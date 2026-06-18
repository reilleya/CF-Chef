use embassy_net::Stack;
use embassy_time::Duration;
use esp_alloc as _;

use core::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};
use picoserve::{
    AppBuilder, AppRouter, Router,
    extract::State,
    response::{File, IntoResponse, IntoResponseWithState, with_state::WithStateUpdate},
    routing,
    routing::{get, post},
};

use crate::constants::MAX_SCHEDULE_STEPS;
use crate::constants::NUM_FANS;
use crate::constants::NUM_THERMOCOUPLES;

#[derive(serde::Deserialize, serde::Serialize)]
pub struct InputScheduleStepValue {
    pub duration: i32,
    pub temperature: i32,
    pub ramp: bool,
}

pub struct InputScheduleStep {
    pub duration: AtomicI32,
    pub temperature: AtomicI32,
    pub ramp: AtomicBool,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct ConfigFormValue {
    max_temp: i32,
    min_temp: i32,
    enabled_tc_zones: i32,  // bitfield, bits correspond to zones
    enabled_fan_zones: i32, // bitfield, bits correspond to fans
    tc_offsets: [i32; NUM_THERMOCOUPLES],
    schedule: [InputScheduleStepValue; MAX_SCHEDULE_STEPS],
}

#[derive(serde::Serialize)]
pub struct ThermocoupleZoneValue {
    enabled: bool,
    last_temp: i32,
    fault: bool,
    offset: i32,
}

#[derive(serde::Serialize)]
pub struct FanValue {
    enabled: bool,
    last_speed: i32,
    fault: bool,
}

#[derive(serde::Serialize)]
struct AppStateValue {
    // Inputs, set by the web interface
    should_start_run: bool,
    schedule: [InputScheduleStepValue; MAX_SCHEDULE_STEPS],
    min_temp: i32,
    max_temp: i32,

    // Outputs, set by the control loop
    temp_zones: [ThermocoupleZoneValue; NUM_THERMOCOUPLES],
    fans: [FanValue; NUM_FANS],
    current_temp: i32,
    current_setpoint: i32,
    run_time_elapsed: i32,
    total_run_time: i32,
    run_state: i32, // 0=Config, 1=Running, 2=Complete, 3=Error
}

pub struct ThermocoupleZone {
    enabled: AtomicBool,
    last_temp: AtomicI32,
    fault: AtomicBool,
    offset: AtomicI32,
}

pub struct Fan {
    enabled: AtomicBool,
    last_speed: AtomicI32,
    fault: AtomicBool,
}

pub struct AppState {
    // Inputs, set by the web interface
    should_start_run: AtomicBool,
    schedule: [InputScheduleStep; MAX_SCHEDULE_STEPS],
    min_temp: AtomicI32,
    max_temp: AtomicI32,

    // Outputs, set by the control loop
    temp_zones: [ThermocoupleZone; NUM_THERMOCOUPLES],
    fans: [Fan; NUM_FANS],
    current_temp: AtomicI32,
    current_setpoint: AtomicI32,
    run_time_elapsed: AtomicI32,
    total_run_time: AtomicI32,
    run_state: AtomicI32,
}

impl picoserve::extract::FromRef<AppState> for AppStateValue {
    fn from_ref(
        AppState {
            schedule,
            current_temp,
            current_setpoint,
            run_time_elapsed,
            total_run_time,
            should_start_run,
            min_temp,
            max_temp,
            temp_zones,
            fans,
            run_state,
            ..
        }: &AppState,
    ) -> Self {
        Self {
            schedule: core::array::from_fn(|i| InputScheduleStepValue {
                duration: schedule[i].duration.load(Relaxed),
                temperature: schedule[i].temperature.load(Relaxed),
                ramp: schedule[i].ramp.load(Relaxed),
            }),
            temp_zones: core::array::from_fn(|i| ThermocoupleZoneValue {
                enabled: temp_zones[i].enabled.load(Relaxed),
                last_temp: temp_zones[i].last_temp.load(Relaxed),
                fault: temp_zones[i].fault.load(Relaxed),
                offset: temp_zones[i].offset.load(Relaxed),
            }),
            fans: core::array::from_fn(|i| FanValue {
                enabled: fans[i].enabled.load(Relaxed),
                last_speed: fans[i].last_speed.load(Relaxed),
                fault: fans[i].fault.load(Relaxed),
            }),
            current_temp: current_temp.load(Relaxed),
            current_setpoint: current_setpoint.load(Relaxed),
            run_time_elapsed: run_time_elapsed.load(Relaxed),
            total_run_time: total_run_time.load(Relaxed),
            should_start_run: should_start_run.load(Relaxed),
            min_temp: min_temp.load(Relaxed),
            max_temp: max_temp.load(Relaxed),
            run_state: run_state.load(Relaxed),
        }
    }
}

async fn get_state(State(value): State<AppStateValue>) -> impl IntoResponse {
    // TODO: only include the "outputs" in the response, not the "inputs"
    picoserve::response::Json(value)
}

async fn set_config(
    picoserve::extract::Json(form_values): picoserve::extract::Json<ConfigFormValue>,
) -> impl IntoResponseWithState<AppState> {
    picoserve::response::Json(0).with_state_update(async move |state: &AppState| {
        // TODO: better response than Json(0) - validate?
        form_values
            .schedule
            .iter()
            .enumerate()
            .for_each(|(i, step)| {
                state.schedule[i].duration.store(step.duration, Relaxed);
                state.schedule[i]
                    .temperature
                    .store(step.temperature, Relaxed);
                state.schedule[i].ramp.store(step.ramp, Relaxed);
            });
        for zone in 0..NUM_THERMOCOUPLES {
            state.temp_zones[zone]
                .enabled
                .store((form_values.enabled_tc_zones & (1 << zone)) != 0, Relaxed);
            state.temp_zones[zone]
                .offset
                .store(form_values.tc_offsets[zone], Relaxed);
        }
        for fan in 0..NUM_FANS {
            state.fans[fan]
                .enabled
                .store((form_values.enabled_fan_zones & (1 << fan)) != 0, Relaxed);
        }
        state.min_temp.store(form_values.min_temp, Relaxed);
        state.max_temp.store(form_values.max_temp, Relaxed);
        state.should_start_run.store(true, Relaxed);
    })
}

pub struct Application;

pub static WEB_STATE: AppState = AppState {
    schedule: [const {
        InputScheduleStep {
            duration: AtomicI32::new(0),
            temperature: AtomicI32::new(0),
            ramp: AtomicBool::new(false),
        }
    }; MAX_SCHEDULE_STEPS],
    temp_zones: [const {
        ThermocoupleZone {
            enabled: AtomicBool::new(false),
            last_temp: AtomicI32::new(0),
            fault: AtomicBool::new(false),
            offset: AtomicI32::new(0),
        }
    }; NUM_THERMOCOUPLES],
    fans: [const {
        Fan {
            enabled: AtomicBool::new(false),
            last_speed: AtomicI32::new(0),
            fault: AtomicBool::new(false),
        }
    }; NUM_FANS],
    current_temp: AtomicI32::new(0),
    current_setpoint: AtomicI32::new(0),
    run_time_elapsed: AtomicI32::new(0),
    total_run_time: AtomicI32::new(0),
    should_start_run: AtomicBool::new(false),
    min_temp: AtomicI32::new(0),
    max_temp: AtomicI32::new(0),
    run_state: AtomicI32::new(0),
};

pub fn set_fan_speed(fan: usize, speed: i32) {
    WEB_STATE.fans[fan].last_speed.store(speed, Relaxed);
}

pub fn set_fan_enabled(fan: usize, enabled: bool) {
    WEB_STATE.fans[fan].enabled.store(enabled, Relaxed);
}

pub fn set_fan_fault(fan: usize, fault: bool) {
    WEB_STATE.fans[fan].fault.store(fault, Relaxed);
}

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

pub fn set_current_setpoint_temperature(value: i32) {
    WEB_STATE.current_setpoint.store(value, Relaxed);
}

pub fn set_total_run_time(value: i32) {
    WEB_STATE.total_run_time.store(value, Relaxed);
}

pub fn set_elapsed_time(value: i32) {
    WEB_STATE.run_time_elapsed.store(value, Relaxed);
}

pub fn set_machine_state(state: &crate::state::State) {
    WEB_STATE.run_state.store(i32::from(state), Relaxed);
}

pub fn should_start_run() -> bool {
    let should_start = WEB_STATE.should_start_run.load(Relaxed);
    // To avoid inadvertently starting a new run when we get back to the config state, reset the flag
    if should_start {
        WEB_STATE.should_start_run.store(false, Relaxed);
    }
    should_start
}

pub fn get_run_config() -> crate::state::RunConfig {
    crate::state::RunConfig::new(
        core::array::from_fn(|i| InputScheduleStepValue {
            duration: WEB_STATE.schedule[i].duration.load(Relaxed),
            temperature: WEB_STATE.schedule[i].temperature.load(Relaxed),
            ramp: WEB_STATE.schedule[i].ramp.load(Relaxed),
        }),
        core::array::from_fn(|i| WEB_STATE.temp_zones[i].enabled.load(Relaxed)),
        core::array::from_fn(|i| WEB_STATE.temp_zones[i].offset.load(Relaxed)),
        core::array::from_fn(|i| WEB_STATE.fans[i].enabled.load(Relaxed)),
        WEB_STATE.min_temp.load(Relaxed),
        WEB_STATE.max_temp.load(Relaxed),
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
