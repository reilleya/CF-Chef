#![no_std]
#![no_main]

use esp_hal::time::Instant;
use firmware as lib;

use core::{net::Ipv4Addr, str::FromStr};

use circular_buffer::CircularBuffer;
use embassy_executor::Spawner;
use embassy_net::{Ipv4Cidr, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};
use esp_alloc as _;
use esp_hal::gpio::{Event, Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::handler;
use esp_hal::interrupt::Priority;
#[cfg(target_arch = "riscv32")]
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::peripherals::Interrupt;
use esp_hal::spi::Mode as SpiMode;
use esp_hal::spi::master::Config as SpiConfig;
use esp_hal::spi::master::Spi;
use esp_hal::time::Rate;
use esp_hal::{clock::CpuClock, ram, rng::Rng, timer::timg::TimerGroup};
use esp_println::println;
use esp_radio::Controller;

use lib::constants::NUM_FANS;
use lib::constants::NUM_THERMOCOUPLES;

esp_bootloader_esp_idf::esp_app_desc!();

#[panic_handler]
fn panic(panic_info: &core::panic::PanicInfo) -> ! {
    println!("PANIC: {}", panic_info.message());
    if let Some(location) = panic_info.location() {
        println!(
            "Panic occurred in file '{}' at line {}",
            location.file(),
            location.line(),
        );
    } else {
        println!("Panic occurred but can't get location information...");
    }

    // TODO: what happens to the GPIO pins here?

    loop {}
}

use core::cell::RefCell;
use critical_section::Mutex;

static FAN_TACHS: Mutex<RefCell<Option<[Input; NUM_FANS]>>> = Mutex::new(RefCell::new(None));
static FAN_PULSE_TIMES: Mutex<RefCell<[CircularBuffer<10, Instant>; NUM_FANS]>> = Mutex::new(
    RefCell::new([const { CircularBuffer::<10, Instant>::new() }; NUM_FANS]),
);

#[handler]
fn handler() {
    critical_section::with(|cs| {
        let mut fan_tachs = FAN_TACHS.borrow_ref_mut(cs);
        let mut fan_pulse_times = FAN_PULSE_TIMES.borrow_ref_mut(cs);

        match fan_tachs.as_mut() {
            Some(fan_tachs) => {
                for fan in 0..NUM_FANS {
                    if fan_tachs[fan].is_interrupt_set() {
                        fan_pulse_times[fan].push_back(Instant::now());
                        fan_tachs[fan].clear_interrupt();
                    }
                }
            }
            _ => {} // If the interrupt gets triggered before setup is done, do nothing
        }
    });
}

const TASK_POOL_SIZE: usize = lib::web::WEB_TASK_POOL_SIZE + lib::wifi::WIFI_TASK_POOL_SIZE;

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

    // Set up LED
    let rmt = esp_hal::rmt::Rmt::new(peripherals.RMT, Rate::from_mhz(80)).unwrap();
    let rmt_output = Output::new(peripherals.GPIO0, Level::Low, Default::default());
    let mut led = lib::led::Led {
        pixel_buffer: [lib::led::Rgb { r: 0, g: 0, b: 0 }; 1],
        rmt_channel: Some(lib::led::Led::configure_rmt(rmt, rmt_output)),
        last_update_time: esp_hal::time::Instant::EPOCH,
    };

    led.set_pixel(0, lib::led::color::WHITE);

    // Set up fan tach interrupts
    esp_hal::interrupt::enable(Interrupt::GPIO, Priority::Priority3).unwrap();

    let mut fan_tachs = [
        Input::new(
            peripherals.GPIO20,
            InputConfig::default().with_pull(Pull::Up),
        ),
        Input::new(
            peripherals.GPIO21,
            InputConfig::default().with_pull(Pull::Up),
        ),
    ];

    critical_section::with(|cs| {
        let mut fan_pulse_times = FAN_PULSE_TIMES.borrow_ref_mut(cs);
        for fan in 0..NUM_FANS {
            fan_tachs[fan].listen(Event::RisingEdge);
            // Add an initial timestamp in case we never hit the interrupt
            fan_pulse_times[fan].push_back(Instant::now());
        }
        FAN_TACHS.borrow_ref_mut(cs).replace(fan_tachs);
    });

    unsafe {
        esp_hal::interrupt::bind_interrupt(Interrupt::GPIO, handler.handler());
    }

    // Set up heater output
    let mut heater_output = Output::new(peripherals.GPIO1, Level::Low, OutputConfig::default());

    // Set up SPI bus for MAX31855 thermocouples
    let mut spi_bus = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(60))
            .with_mode(SpiMode::_0),
    )
    .unwrap()
    .with_sck(peripherals.GPIO5)
    .with_miso(peripherals.GPIO10);
    let mut spi_cs_pins: [Output; lib::constants::NUM_THERMOCOUPLES] = [
        Output::new(peripherals.GPIO4, Level::High, OutputConfig::default()),
        Output::new(peripherals.GPIO6, Level::High, OutputConfig::default()),
        Output::new(peripherals.GPIO7, Level::High, OutputConfig::default()),
    ];

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    #[cfg(target_arch = "riscv32")]
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(
        timg0.timer0,
        #[cfg(target_arch = "riscv32")]
        sw_int.software_interrupt0,
    );

    let esp_radio_ctrl = &*lib::mk_static!(Controller<'static>, esp_radio::init().unwrap());

    let (controller, interfaces) =
        esp_radio::wifi::new(&esp_radio_ctrl, peripherals.WIFI, Default::default()).unwrap();

    let device = interfaces.ap;

    let gw_ip_addr = Ipv4Addr::from_str(lib::wifi::GW_IP_ADDR).expect("failed to parse gateway ip");

    let config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(gw_ip_addr, 24),
        gateway: Some(gw_ip_addr),
        dns_servers: Default::default(),
    });

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;

    // Init network stack
    let (stack, runner) = embassy_net::new(
        device,
        config,
        lib::mk_static!(StackResources<TASK_POOL_SIZE>, StackResources::new()),
        seed,
    );

    spawner.spawn(lib::wifi::connection(controller)).ok();
    spawner.spawn(lib::wifi::net_task(runner)).ok();
    spawner.spawn(lib::wifi::run_dhcp(stack)).ok();

    loop {
        if stack.is_link_up() {
            break;
        }
        Timer::after(Duration::from_millis(500)).await;
    }

    let ssid = lib::wifi::SSID;
    let gw_ip_addr_str = lib::wifi::GW_IP_ADDR;
    println!("Connect to the AP `{ssid}` and point your browser to http://{gw_ip_addr_str}");
    println!("DHCP is enabled so there's no need to configure a static IP, just in case:");
    while !stack.is_config_up() {
        Timer::after(Duration::from_millis(100)).await
    }
    stack
        .config_v4()
        .inspect(|c| println!("ipv4 config: {c:?}"));

    let web_app = lib::web::WebApp::default();
    for id in 0..lib::web::WEB_TASK_POOL_SIZE {
        spawner.must_spawn(lib::web::web_task(
            id,
            stack,
            web_app.router,
            web_app.config,
        ));
    }

    let mut state = lib::state::State::Config;

    led.set_pixel(0, lib::led::color::OFF);

    'main: loop {
        let mut temperatures = [0.0; NUM_THERMOCOUPLES];
        let mut tc_faults = [true; NUM_THERMOCOUPLES];
        for zone in 0..NUM_THERMOCOUPLES {
            spi_cs_pins[zone].set_low();
            let mut buffer = [0; 4];
            spi_bus.read(&mut buffer).unwrap();
            spi_cs_pins[zone].set_high();

            let reading = lib::max31855::interpret_max31855_read(buffer);
            //lib::max31855::log_max31855_reading(&reading);
            if let lib::max31855::MAX31855Reading::Valid { temp, .. } = reading {
                // If we have a run in progress, pull TC offsets from it and apply them
                let offset: i32 = match state {
                    lib::state::State::Running {
                        ref config,
                        run_start_time: _,
                    } => config.tc_offsets[zone],
                    _ => 0,
                };
                let adjusted = temp + offset as f32;
                lib::web::set_zone_temperature(zone, adjusted as i32);
                temperatures[zone] = adjusted;
                tc_faults[zone] = false;
            }
            lib::web::set_zone_fault(zone, tc_faults[zone]);
        }

        let mut fan_faults = [false; NUM_FANS];
        critical_section::with(|cs| {
            let fan_pulse_times = FAN_PULSE_TIMES.borrow_ref(cs);

            for fan in 0..NUM_FANS {
                match fan_pulse_times[fan].back() {
                    Some(last_pulse) => {
                        if last_pulse.elapsed().as_secs() > 1 {
                            fan_faults[fan] = true;
                        }
                    }
                    None => {
                        // This shouldn't happen since we add a timestamp on startup, so if it does, that's definitely a fault
                        fan_faults[fan] = true;
                    }
                }
                lib::web::set_fan_fault(fan, fan_faults[fan]);

                let mut fan_total_period = 0.0;
                for period in fan_pulse_times[fan]
                    .iter()
                    .zip(fan_pulse_times[fan].iter().skip(1))
                {
                    fan_total_period += (period.1.duration_since_epoch()
                        - period.0.duration_since_epoch())
                    .as_millis() as f32;
                }
                let fan_rpm =
                    60000.0 / (fan_total_period / (fan_pulse_times[fan].len() - 1) as f32) / 2.0; // 2 pulses per revolution
                lib::web::set_fan_speed(fan, fan_rpm as i32);
            }
        });

        match state {
            lib::state::State::Config => {
                if lib::web::should_start_run() {
                    state = lib::state::State::Running {
                        config: lib::web::get_run_config(),
                        run_start_time: esp_hal::time::Instant::now(),
                    }
                }
            }
            lib::state::State::Running {
                ref config,
                run_start_time,
            } => {
                // TODO: why can't I just get the config?
                led.set_pixel(0, lib::led::color::GREEN); // TODO: use a lookup table to set color based on state

                // Check for TC faults
                for zone in 0..NUM_THERMOCOUPLES {
                    if !config.enabled_tc_zones[zone] {
                        continue;
                    }

                    if tc_faults[zone] {
                        state = lib::state::State::Error {
                            reason: lib::state::RunFailureReason::ThermocoupleFault { zone },
                        };
                        continue 'main;
                    }

                    if temperatures[zone] as i32 <= config.min_temp {
                        state = lib::state::State::Error {
                            reason: lib::state::RunFailureReason::UnderTempFault { zone },
                        };
                        continue 'main;
                    }

                    if temperatures[zone] as i32 >= config.max_temp {
                        state = lib::state::State::Error {
                            reason: lib::state::RunFailureReason::OverTempFault { zone },
                        };
                        continue 'main;
                    }
                }

                // Check for fan faults
                for fan in 0..NUM_FANS {
                    if config.fan_enabled[fan] && fan_faults[fan] {
                        state = lib::state::State::Error {
                            reason: lib::state::RunFailureReason::FanFault { number: fan },
                        };
                        continue 'main;
                    }
                }

                // Check if run is complete
                let elapsed = esp_hal::time::Instant::now() - run_start_time;
                lib::web::set_elapsed_time(elapsed.as_secs() as i32);
                if elapsed >= config.get_total_run_time() {
                    state = lib::state::State::Complete;
                    continue 'main;
                }

                let current_setpoint = config.get_setpoint_for_time(elapsed);

                // All expected TCs have good readings, calculate average temperature
                let average_temp: f32 = config
                    .enabled_tc_zones
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &enabled)| if enabled { Some(temperatures[i]) } else { None })
                    .sum::<f32>()
                    / config
                        .enabled_tc_zones
                        .iter()
                        .filter(|&&enabled| enabled)
                        .count() as f32;

                lib::web::set_current_temperature(average_temp as i32);
                lib::web::set_current_setpoint_temperature(current_setpoint as i32);

                // Bang-bang control
                if average_temp < current_setpoint {
                    heater_output.set_high();
                } else {
                    heater_output.set_low();
                }
            }
            lib::state::State::Complete => {
                heater_output.set_low();
                led.set_pixel(0, lib::led::color::PURPLE);
            }
            lib::state::State::Error { ref reason } => {
                heater_output.set_low();
                led.set_pixel(0, lib::led::color::RED);
                println!("Run failed: {reason:?}");
            }
        }
        lib::web::set_machine_state(&state);

        Timer::after(Duration::from_millis(100)).await;
    }
}
