use embassy_net::Stack;
use embassy_time::Duration;
use esp_alloc as _;

use core::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};
use picoserve::{
    AppBuilder, AppRouter, Router,
    extract::Form,
    extract::State,
    response::{File, IntoResponse, IntoResponseWithState, Redirect, with_state::WithStateUpdate},
    routing,
    routing::{get, parse_path_segment, post},
};

#[derive(serde::Deserialize)]
struct FormValue {
    temperature: i32,
    time: i32,
}

pub struct AppState {
    current_temp: AtomicI32,
    setpoint_temp: AtomicI32,
    run_time_elapsed: AtomicI32,
    run_time_total: AtomicI32,
    run_started: AtomicBool,
}

#[derive(serde::Serialize)]
struct AppStateValue {
    current_temp: i32,
    setpoint_temp: i32,
    run_time_elapsed: i32,
    run_time_total: i32,
    run_started: bool,
}

impl picoserve::extract::FromRef<AppState> for AppStateValue {
    fn from_ref(
        AppState {
            current_temp,
            setpoint_temp,
            run_time_elapsed,
            run_time_total,
            run_started,
            ..
        }: &AppState,
    ) -> Self {
        Self {
            current_temp: current_temp.load(Relaxed),
            setpoint_temp: setpoint_temp.load(Relaxed),
            run_time_elapsed: run_time_elapsed.load(Relaxed),
            run_time_total: run_time_total.load(Relaxed),
            run_started: run_started.load(Relaxed),
        }
    }
}

async fn get_state(State(value): State<AppStateValue>) -> impl IntoResponse {
    picoserve::response::Json(value)
}

async fn increment_temperature() -> impl IntoResponseWithState<AppState> {
    Redirect::to(".").with_state_update(async |state: &AppState| {
        // TODO: use fetch_add
        let old_setpoint = state.setpoint_temp.load(Relaxed);
        state.setpoint_temp.store(old_setpoint + 1, Relaxed);
    })
}

async fn set_temperature(value: i32) -> impl IntoResponseWithState<AppState> {
    Redirect::to("..").with_state_update(async move |state: &AppState| {
        state.setpoint_temp.store(value, Relaxed);
    })
}

async fn set_config(
    Form(FormValue { temperature, time }): Form<FormValue>,
) -> impl IntoResponseWithState<AppState> {
    picoserve::response::Json(0).with_state_update(async move |state: &AppState| {
        // TODO: better response than Json(0)
        state.setpoint_temp.store(temperature, Relaxed); // TODO: validate?
        state.run_time_total.store(time, Relaxed);
        state.run_started.store(true, Relaxed);
    })
}

pub struct Application;

pub static STATE: AppState = AppState {
    current_temp: AtomicI32::new(0),
    setpoint_temp: AtomicI32::new(0),
    run_time_elapsed: AtomicI32::new(0),
    run_time_total: AtomicI32::new(0),
    run_started: AtomicBool::new(false),
};

pub fn set_current_temperature(value: i32) {
    STATE.current_temp.store(value, Relaxed);
}

pub fn set_elapsed_time(value: i32) {
    STATE.run_time_elapsed.store(value, Relaxed);
}

pub fn get_run_started() -> bool {
    STATE.run_started.load(Relaxed)
}

pub fn get_run_total_time() -> i32 {
    STATE.run_time_total.load(Relaxed)
}

pub fn get_setpoint_temperature() -> i32 {
    STATE.setpoint_temp.load(Relaxed)
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
            .route("/increment", get(increment_temperature))
            .route(("/set", parse_path_segment()), get(set_temperature))
            .route("/get_state", get(get_state))
            .route("/set_config", post(set_config))
            .with_state(&STATE)
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
