use embassy_net::Stack;
use embassy_time::Duration;
use esp_alloc as _;

use core::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};
use picoserve::{
    AppBuilder, AppRouter, Router,
    extract::Form,
    extract::State,
    response::{File, IntoResponse, IntoResponseWithState, with_state::WithStateUpdate},
    routing,
    routing::{get, post},
};

#[derive(serde::Deserialize)]
struct RunConfig {
    temperature: i32,
    time: i32,
    enabled_tc_zones: i32, // bitfield, bits correspond to zones
}

#[derive(serde::Serialize)]
pub struct ThermocoupleZoneValue {
    enabled: bool,
    last_temp: i32,
    fault: bool,
}

#[derive(serde::Serialize)]
struct AppStateValue {
    // Inputs, set by the web interface
    run_started: bool,
    setpoint_temp: i32,
    run_time_total: i32,

    // Outputs, set by the control loop
    temp_zones: [ThermocoupleZoneValue; 3],
    current_temp: i32,
    run_time_elapsed: i32,
}

pub struct ThermocoupleZone {
    enabled: AtomicBool,
    last_temp: AtomicI32,
    fault: AtomicBool,
}

pub struct AppState {
    // Inputs, set by the web interface
    run_started: AtomicBool,
    setpoint_temp: AtomicI32,
    run_time_total: AtomicI32,

    // Outputs, set by the control loop
    temp_zones: [ThermocoupleZone; 3],
    current_temp: AtomicI32,
    run_time_elapsed: AtomicI32,
}

impl picoserve::extract::FromRef<AppState> for AppStateValue {
    fn from_ref(
        AppState {
            current_temp,
            setpoint_temp,
            run_time_elapsed,
            run_time_total,
            run_started,
            temp_zones,
            ..
        }: &AppState,
    ) -> Self {
        Self {
            temp_zones: [
                ThermocoupleZoneValue {
                    enabled: temp_zones[0].enabled.load(Relaxed),
                    last_temp: temp_zones[0].last_temp.load(Relaxed),
                    fault: temp_zones[0].fault.load(Relaxed),
                },
                ThermocoupleZoneValue {
                    enabled: temp_zones[1].enabled.load(Relaxed),
                    last_temp: temp_zones[1].last_temp.load(Relaxed),
                    fault: temp_zones[1].fault.load(Relaxed),
                },
                ThermocoupleZoneValue {
                    enabled: temp_zones[2].enabled.load(Relaxed),
                    last_temp: temp_zones[2].last_temp.load(Relaxed),
                    fault: temp_zones[2].fault.load(Relaxed),
                },
            ],
            current_temp: current_temp.load(Relaxed),
            setpoint_temp: setpoint_temp.load(Relaxed),
            run_time_elapsed: run_time_elapsed.load(Relaxed),
            run_time_total: run_time_total.load(Relaxed),
            run_started: run_started.load(Relaxed),
        }
    }
}

async fn get_state(State(value): State<AppStateValue>) -> impl IntoResponse { // TODO: only include the "outputs" in the response, not the "inputs"
    picoserve::response::Json(value)
}

async fn set_config(
    Form(RunConfig { temperature, time, enabled_tc_zones }): Form<RunConfig>,
) -> impl IntoResponseWithState<AppState> {
    picoserve::response::Json(0).with_state_update(async move |state: &AppState| {
        // TODO: better response than Json(0) - validate?
        state.setpoint_temp.store(temperature, Relaxed); // TODO: validate?
        state.run_time_total.store(time, Relaxed);
        state.temp_zones[0].enabled.store((enabled_tc_zones & 0x1) != 0, Relaxed);
        state.temp_zones[1].enabled.store((enabled_tc_zones & 0x2) != 0, Relaxed);
        state.temp_zones[2].enabled.store((enabled_tc_zones & 0x4) != 0, Relaxed);
        state.run_started.store(true, Relaxed);
    })
}

pub struct Application;

pub static WEB_STATE: AppState = AppState {
    temp_zones: [
        ThermocoupleZone {
            enabled: AtomicBool::new(false),
            last_temp: AtomicI32::new(0),
            fault: AtomicBool::new(false),
        },
        ThermocoupleZone {
            enabled: AtomicBool::new(false),
            last_temp: AtomicI32::new(0),
            fault: AtomicBool::new(false),
        },
        ThermocoupleZone {
            enabled: AtomicBool::new(false),
            last_temp: AtomicI32::new(0),
            fault: AtomicBool::new(false),
        },
    ],
    current_temp: AtomicI32::new(0),
    setpoint_temp: AtomicI32::new(0),
    run_time_elapsed: AtomicI32::new(0),
    run_time_total: AtomicI32::new(0),
    run_started: AtomicBool::new(false),
};

pub fn set_zone_temperature(zone: usize, value: i32) {
    WEB_STATE.temp_zones[zone].last_temp.store(value, Relaxed);
}

pub fn set_zone_enabled(zone: usize, enabled: bool) {
    WEB_STATE.temp_zones[zone].enabled.store(enabled, Relaxed);
}

pub fn set_zone_fault(zone: usize, fault: bool) {
    WEB_STATE.temp_zones[zone].fault.store(fault, Relaxed);
}

pub fn set_current_temperature(value: i32) {
    WEB_STATE.current_temp.store(value, Relaxed);
}

pub fn set_elapsed_time(value: i32) {
    WEB_STATE.run_time_elapsed.store(value, Relaxed);
}

pub fn get_run_started() -> bool {
    let started = WEB_STATE.run_started.load(Relaxed);
    // To avoid inadvertently starting a new run when we get back to the config state, reset the flag
    if started {
        WEB_STATE.run_started.store(false, Relaxed);
    }
    started
}

pub fn get_run_config() -> crate::state::RunConfig {
    crate::state::RunConfig::new(
        WEB_STATE.setpoint_temp.load(Relaxed),
        WEB_STATE.run_time_total.load(Relaxed),
        [
            WEB_STATE.temp_zones[0].enabled.load(Relaxed),
            WEB_STATE.temp_zones[1].enabled.load(Relaxed),
            WEB_STATE.temp_zones[2].enabled.load(Relaxed),
        ],
    )
}

impl AppBuilder for Application {
    type PathRouter = impl routing::PathRouter;

    fn build_app(self) -> picoserve::Router<Self::PathRouter> {
        picoserve::Router::new()
            .route(
                "/",
                routing::get_service(File::html(include_str!("web/index.html"))),
            )
            .route(
                "/buttons.js",
                routing::get_service(File::javascript(include_str!("web/buttons.js"))),
            )
            .route(
                "/styles.css",
                routing::get_service(File::css(include_str!("web/styles.css"))),
            )
            .route("/get_state", get(get_state))
            .route("/set_config", post(set_config))
            .with_state(&WEB_STATE)
    }
}

pub const WEB_TASK_POOL_SIZE: usize = 2;

#[embassy_executor::task(pool_size = WEB_TASK_POOL_SIZE)]
pub async fn web_task(
    task_id: usize,
    stack: Stack<'static>,
    router: &'static AppRouter<Application>,
    config: &'static picoserve::Config<Duration>,
) -> ! {
    let port = 80;
    let mut tcp_rx_buffer = [0; 1024];
    let mut tcp_tx_buffer = [0; 1024];
    let mut http_buffer = [0; 2048];

    picoserve::Server::new(router, config, &mut http_buffer)
        .listen_and_serve(task_id, stack, port, &mut tcp_rx_buffer, &mut tcp_tx_buffer)
        .await
        .into_never()
}

pub struct WebApp {
    pub router: &'static Router<<Application as AppBuilder>::PathRouter>,
    pub config: &'static picoserve::Config<Duration>,
}

impl Default for WebApp {
    fn default() -> Self {
        let router = picoserve::make_static!(AppRouter<Application>, Application.build_app());

        let config = picoserve::make_static!(
            picoserve::Config<Duration>,
            picoserve::Config::new(picoserve::Timeouts {
                start_read_request: Some(Duration::from_secs(5)),
                read_request: Some(Duration::from_secs(1)),
                write: Some(Duration::from_secs(1)),
                persistent_start_read_request: Some(Duration::from_secs(1)),
            })
            .keep_connection_alive()
        );

        Self { router, config }
    }
}
