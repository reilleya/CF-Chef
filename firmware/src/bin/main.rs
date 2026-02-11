#![no_std]
#![no_main]

use firmware as lib;

use core::{net::Ipv4Addr, str::FromStr};

use embassy_executor::Spawner;
use embassy_net::{Ipv4Cidr, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};
use esp_alloc as _;
use esp_hal::gpio::{Level, Output, OutputConfig};
#[cfg(target_arch = "riscv32")]
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::spi::Mode as SpiMode;
use esp_hal::spi::master::Config as SpiConfig;
use esp_hal::spi::master::Spi;
use esp_hal::time::Rate;
use esp_hal::{clock::CpuClock, ram, rng::Rng, timer::timg::TimerGroup};
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
    let mut leds = lib::led::Led {
        pixel_buffer: [lib::led::Rgb { r: 0, g: 0, b: 0 }; 1],
        rmt_channel: Some(lib::led::Led::configure_rmt(rmt, rmt_output)),
        last_update_time: esp_hal::time::Instant::EPOCH,
    };

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

    let mut started = false;
    let mut ended = false;
    let mut setpoint_temp = 0;
    let mut run_start_time = esp_hal::time::Instant::now();
    let mut run_duration = esp_hal::time::Duration::from_secs(0);

    leds.set_pixel(0, lib::led::color::OFF);

    loop {
        let mut temperatures = [0.0; 3];
        let enabled_zones = [true, false, false]; // TODO: add zone enable/disable to configuration form
        for zone in 0..3 {
            spi_cs_pins[zone].set_low();
            let mut buffer = [0; 4];
            spi_bus.read(&mut buffer).unwrap();
            spi_cs_pins[zone].set_high();

            let reading = lib::max31855::interpret_max31855_read(buffer);
            lib::max31855::log_max31855_reading(&reading);
            if let lib::max31855::MAX31855Reading::Valid { temp, .. } = reading {
                lib::web::set_zone_temperature(zone, temp as i32);
                temperatures[zone] = temp;
            }
            // TODO: complain if there is a fault reading and this zone is enabled
        }

        let average_temp: f32 = enabled_zones
            .iter()
            .enumerate()
            .filter_map(|(i, &enabled)| if enabled { Some(temperatures[i]) } else { None })
            .sum::<f32>()
            / enabled_zones.iter().filter(|&&enabled| enabled).count() as f32;

        lib::web::set_current_temperature(average_temp as i32);
        println!("Average temperature: {average_temp:.2} °C");
        println!("");

        if !started {
            if lib::web::get_run_started() {
                run_start_time = esp_hal::time::Instant::now();
                started = true;
                setpoint_temp = lib::web::get_setpoint_temperature();
                run_duration =
                    esp_hal::time::Duration::from_secs(lib::web::get_run_total_time() as u64);
            }
        } else {
            let elapsed = esp_hal::time::Instant::now() - run_start_time;
            if elapsed >= run_duration {
                ended = true;
            }
            if ended {
                heater_output.set_low();
                leds.set_pixel(0, lib::led::color::PURPLE);
            } else {
                lib::web::set_elapsed_time(elapsed.as_secs() as i32);
                leds.set_pixel(0, lib::led::color::GREEN);
                if average_temp < setpoint_temp as f32 {
                    heater_output.set_high();
                } else {
                    heater_output.set_low();
                }
            }
        }

        Timer::after(Duration::from_millis(100)).await;
    }
}
