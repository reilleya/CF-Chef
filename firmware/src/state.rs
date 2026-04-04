pub struct RunConfig {
    pub temperature: i32,
    pub duration: esp_hal::time::Duration,
    pub enabled_tc_zones: [bool; 3],
    pub fan_enabled: [bool; 2],
}

impl RunConfig {
    pub fn new(
        temperature: i32,
        time_seconds: i32,
        enabled_tc_zones: [bool; 3],
        fan_enabled: [bool; 2],
    ) -> Self {
        Self {
            temperature,
            duration: esp_hal::time::Duration::from_secs(time_seconds as u64),
            enabled_tc_zones,
            fan_enabled,
        }
    }
}

pub enum RunFailureReason {
    ThermocoupleFault { zone: usize },
    FanFault { number: usize },
}

impl core::fmt::Debug for RunFailureReason {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            RunFailureReason::ThermocoupleFault { zone } => {
                write!(f, "Thermocouple fault in zone {}", zone)
            }
            RunFailureReason::FanFault { number } => write!(f, "Fan {} had a fault", number),
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
        reason: RunFailureReason, // TODO: should there be a way to have multiple failures so they know what hardware to fix?
    },
}
