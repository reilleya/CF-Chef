#![no_std]
#![no_main]

use esp_hal::time::Instant;
use firmware as lib;

use core::{net::Ipv4Addr, str::FromStr};

use embassy_executor::Spawner;
use embassy_net::{Ipv4Cidr, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};
use esp_alloc as _;
use esp_hal::gpio::{Event, Level, Output, OutputConfig, Input, InputConfig, Io, Pull};
#[cfg(target_arch = "riscv32")]
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::spi::Mode as SpiMode;
use esp_hal::spi::master::Config as SpiConfig;
use esp_hal::spi::master::Spi;
use esp_hal::time::Rate;
use esp_hal::{clock::CpuClock, ram, rng::Rng, timer::timg::TimerGroup};
use esp_hal::interrupt::Priority;
use esp_hal::peripherals::Interrupt;
use esp_hal::handler;
use esp_println::println;
use esp_radio::Controller;

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
static FAN0_TACH: Mutex<RefCell<Option<Input>>> =
    Mutex::new(RefCell::new(None));

static FAN1_TACH: Mutex<RefCell<Option<Input>>> =
    Mutex::new(RefCell::new(None));

static FAN0_LAST_PULSE: Mutex<RefCell<Option<Instant>>> =
    Mutex::new(RefCell::new(None));

static FAN1_LAST_PULSE: Mutex<RefCell<Option<Instant>>> =
    Mutex::new(RefCell::new(None));

#[handler]
fn handler() {
    critical_section::with(|cs| {
        let mut fan0_tach = FAN0_TACH.borrow_ref_mut(cs);
        let mut fan1_tach = FAN1_TACH.borrow_ref_mut(cs);
        let mut fan0_last_pulse = FAN0_LAST_PULSE.borrow_ref_mut(cs);
        let mut fan1_last_pulse = FAN1_LAST_PULSE.borrow_ref_mut(cs);
        match (fan0_tach.as_mut(), fan0_last_pulse.as_mut()) {
            (Some(fan0_tach), Some(fan0_last_pulse)) => {
                if fan0_tach.is_interrupt_set() {
                    println!("Got a pulse on 0, {} ms since last!", fan0_last_pulse.elapsed().as_millis());

                    *fan0_last_pulse = Instant::now();
                    fan0_tach.clear_interrupt();
                }
            }
            _ => { }
        }
        match (fan1_tach.as_mut(), fan1_last_pulse.as_mut()) {
            (Some(fan1_tach), Some(fan1_last_pulse)) => {
                if fan1_tach.is_interrupt_set() {
                    println!("Got a pulse on 1, {} ms since last!", fan1_last_pulse.elapsed().as_millis());

                    *fan1_last_pulse = Instant::now();
                    fan1_tach.clear_interrupt();
                }
            }
            _ => { }
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

    // Set up fan tach interrupts
    esp_hal::interrupt::enable(Interrupt::GPIO, Priority::Priority3).unwrap();

    let mut fan0_tach = Input::new(peripherals.GPIO20, InputConfig::default().with_pull(Pull::Up));
    let mut fan1_tach = Input::new(peripherals.GPIO21, InputConfig::default().with_pull(Pull::Up));

    critical_section::with(|cs| {
        fan0_tach.listen(Event::RisingEdge);
        fan1_tach.listen(Event::RisingEdge);
        FAN0_TACH.borrow_ref_mut(cs).replace(fan0_tach);
        FAN1_TACH.borrow_ref_mut(cs).replace(fan1_tach);
        FAN0_LAST_PULSE.borrow_ref_mut(cs).replace(Instant::now());
        FAN1_LAST_PULSE.borrow_ref_mut(cs).replace(Instant::now());
    });

    unsafe {
        esp_hal::interrupt::bind_interrupt(Interrupt::GPIO, handler.handler());
    }


    // Set up LED
    let rmt = esp_hal::rmt::Rmt::new(peripherals.RMT, Rate::from_mhz(80)).unwrap();
    let rmt_output = Output::new(peripherals.GPIO0, Level::Low, Default::default());
    let mut led = lib::led::Led {
        pixel_buffer: [lib::led::Rgb { r: 0, g: 0, b: 0 }; 1],
        rmt_channel: Some(lib::led::Led::configure_rmt(rmt, rmt_output)),
        last_update_time: esp_hal::time::Instant::EPOCH,
    };

    led.set_pixel(0, lib::led::color::WHITE);

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
    let mut spi_cs_pins = [
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
        let mut temperatures = [0.0; 3];
        let mut faults = [false; 3];
        for zone in 0..3 {
            spi_cs_pins[zone].set_low();
            let mut buffer = [0; 4];
            spi_bus.read(&mut buffer).unwrap();
            spi_cs_pins[zone].set_high();

            let reading = lib::max31855::interpret_max31855_read(buffer);
            //lib::max31855::log_max31855_reading(&reading);
            if let lib::max31855::MAX31855Reading::Valid { temp, .. } = reading {
                lib::web::set_zone_temperature(zone, temp as i32);
                temperatures[zone] = temp;
            }
            else {
                lib::web::set_zone_fault(zone, true);
                faults[zone] = true;
            }
        }

        critical_section::with(|cs| {
            lib::web::set_last_tach_pulse_time(0, FAN0_LAST_PULSE.borrow_ref_mut(cs).unwrap().elapsed().as_millis() as i32);
            lib::web::set_last_tach_pulse_time(1, FAN1_LAST_PULSE.borrow_ref_mut(cs).unwrap().elapsed().as_millis() as i32);
        });

        match state {
            lib::state::State::Config => {
                if lib::web::get_run_started() {
                    state = lib::state::State::Running {
                        config: lib::web::get_run_config(),
                        run_start_time: esp_hal::time::Instant::now(),
                    }
                }
            }
            lib::state::State::Running { ref config, run_start_time } => { // TODO: why can't I just get the config?
                led.set_pixel(0, lib::led::color::GREEN); // TODO: use a lookup table to set color based on state

                // Check for TC faults
                for zone in 0..3 {
                    if config.enabled_tc_zones[zone] && faults[zone] {
                        state = lib::state::State::Error {
                            reason: lib::state::RunFailureReason::ThermocoupleFault { zone },
                        };
                        continue 'main;
                    }
                }

                // Check if run is complete
                let elapsed = esp_hal::time::Instant::now() - run_start_time;
                lib::web::set_elapsed_time(elapsed.as_secs() as i32);
                if elapsed >= config.duration {
                    state = lib::state::State::Complete;
                    continue 'main;
                }

                // All expected TCs have good readings, calculate average temperature
                let average_temp: f32 = config.enabled_tc_zones
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &enabled)| if enabled { Some(temperatures[i]) } else { None })
                    .sum::<f32>()
                    / config.enabled_tc_zones.iter().filter(|&&enabled| enabled).count() as f32;

                lib::web::set_current_temperature(average_temp as i32);

                // Bang-bang control
                if average_temp < config.temperature as f32 {
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

        Timer::after(Duration::from_millis(100)).await;
    }
}
