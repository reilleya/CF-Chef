pub struct RunConfig {
    pub temperature: i32,
    pub duration: esp_hal::time::Duration,
}

impl RunConfig {
    pub fn new(temperature: i32, time_seconds: i32) -> Self {
        Self {
            temperature,
            duration: esp_hal::time::Duration::from_secs(time_seconds as u64),
        }
    }
}

pub enum State {
    Config,
    Running {
        config: RunConfig,
        run_start_time: esp_hal::time::Instant,
    },
    Complete,
    Error {
        message: &'static str,
    }
}


/*pub struct StateMachineInput {
    state: State,
    run_started: bool,

}

pub struct StateMachineOutput {
    state: State,
    heater_enabled: bool,

}
static mut state: State = State::Config;

pub fn next_state(input: StateMachineInput) -> StateMachineOutput {
    match input.state {
        State::Config => {
            if input.run_started {
                return StateMachineOutput {
                    state: State::Running {
                        start_time: 0,
                        duration_secs: 0,
                    },
                    heater_enabled: false,
                }
            }
            // if web state says run started
                // Set up run using parameters from web state
                // State::Running
            // else
                // State::Config
            StateMachineOutput {
                state: State::Config,
                heater_enabled: false,
            }
        }
        State::Running { start_time, duration_secs } => {
            // Check for faults
                // State::Error
            // Update run from 
            StateMachineOutput {
                state: State::Complete,
                heater_enabled: false,
            }
        }
        State::Complete => {
            StateMachineOutput {
                state: State::Complete,
                heater_enabled: false,
            }
        }
        State::Error => {
            StateMachineOutput {
                state: State::Error,
                heater_enabled: false,
            }
        }
    }
}*/