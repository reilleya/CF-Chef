#![no_std]
#![no_main]

use firmware as lib;

use core::{net::Ipv4Addr, str::FromStr};

use embassy_executor::Spawner;
use embassy_net::{Ipv4Cidr, StackResources, StaticConfigV4};
use embassy_time::{Duration, Timer};
use esp_alloc as _;
//use esp_backtrace as _;
#[cfg(target_arch = "riscv32")]
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::{clock::CpuClock, ram, rng::Rng, timer::timg::TimerGroup};
use esp_hal::time::Rate;
use esp_hal::gpio::{self, Input, InputConfig, Level, Output, OutputConfig};
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

    loop {}
}

const TASK_POOL_SIZE: usize = lib::web::WEB_TASK_POOL_SIZE + lib::wifi::WIFI_TASK_POOL_SIZE;

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    esp_println::logger::init_logger_from_env();
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let rmt = esp_hal::rmt::Rmt::new(peripherals.RMT, Rate::from_mhz(80)).unwrap();
    let rmt_output = Output::new(peripherals.GPIO0, Level::Low, Default::default());
    let mut leds = lib::led::Led {
        pixel_buffer: [lib::led::Rgb { r: 0, g: 0, b: 0 }; 1],
        rmt_channel: Some(lib::led::Led::configure_rmt(rmt, rmt_output)),
        last_update_time: esp_hal::time::Instant::EPOCH,
    };

    esp_alloc::heap_allocator!(#[ram(reclaimed)] size: 64 * 1024);
    esp_alloc::heap_allocator!(size: 36 * 1024);

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

    loop {
        leds.set_pixel(0, lib::led::color::GREEN);
        Timer::after(Duration::from_secs(1)).await;
        leds.set_pixel(0, lib::led::color::RED);
        Timer::after(Duration::from_secs(1)).await;
    }
}
