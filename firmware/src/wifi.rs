use core::str::FromStr;

use embassy_net::{Runner, Stack};
use embassy_time::{Duration, Timer};
use esp_alloc as _;
//use esp_backtrace as _;
#[cfg(target_arch = "riscv32")]
use esp_println::println;
use esp_radio::wifi::{
    AccessPointConfig, AuthMethod, ModeConfig, WifiApState, WifiController, WifiDevice, WifiEvent,
};

pub const WIFI_TASK_POOL_SIZE: usize = 3;

pub const SSID: &'static str = "CF-Chef";
pub const PASSWORD: &'static str = "time2cook";
pub const GW_IP_ADDR: &'static str = "192.168.2.1";

#[embassy_executor::task]
pub async fn run_dhcp(stack: Stack<'static>) {
    use core::net::{Ipv4Addr, SocketAddrV4};

    use edge_dhcp::{
        io::{self, DEFAULT_SERVER_PORT},
        server::{Server, ServerOptions},
    };
    use edge_nal::UdpBind;
    use edge_nal_embassy::{Udp, UdpBuffers};

    let gw_ip_addr = Ipv4Addr::from_str(GW_IP_ADDR).expect("failed to parse gateway ip");

    let mut buf = [0u8; 1500];

    let mut gw_buf = [Ipv4Addr::UNSPECIFIED];

    let buffers = UdpBuffers::<3, 1024, 1024, 10>::new();
    let unbound_socket = Udp::new(stack, &buffers);
    let mut bound_socket = unbound_socket
        .bind(core::net::SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            DEFAULT_SERVER_PORT,
        )))
        .await
        .unwrap();

    loop {
        _ = io::server::run(
            &mut Server::<_, 64>::new_with_et(gw_ip_addr),
            &ServerOptions::new(gw_ip_addr, Some(&mut gw_buf)),
            &mut bound_socket,
            &mut buf,
        )
        .await
        .inspect_err(|e| println!("DHCP server error: {e:?}"));
        Timer::after(Duration::from_millis(500)).await;
    }
}

#[embassy_executor::task]
pub async fn connection(mut controller: WifiController<'static>) {
    println!("start connection task");
    println!("Device capabilities: {:?}", controller.capabilities());
    loop {
        match esp_radio::wifi::ap_state() {
            WifiApState::Started => {
                // wait until we're no longer connected
                controller.wait_for_event(WifiEvent::ApStop).await;
                Timer::after(Duration::from_millis(5000)).await
            }
            _ => {}
        }
        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = ModeConfig::AccessPoint(
                AccessPointConfig::default()
                    .with_ssid(SSID.into())
                    .with_auth_method(AuthMethod::Wpa2Personal)
                    .with_password(PASSWORD.into()),
            );
            controller.set_config(&client_config).unwrap();
            println!("Starting wifi");
            controller.start_async().await.unwrap();
            println!("Wifi started!");
        }
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}
