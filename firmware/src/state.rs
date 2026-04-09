use crate::constants::MAX_SCHEDULE_STEPS;
use crate::constants::NUM_FANS;
use crate::constants::NUM_THERMOCOUPLES;
use crate::web::InputScheduleStepValue;

pub struct ScheduleStep {
    pub duration: esp_hal::time::Duration,
    pub temperature: i32,
    pub ramp: bool,
}

pub struct RunConfig {
    pub schedule: [ScheduleStep; MAX_SCHEDULE_STEPS],
    pub enabled_tc_zones: [bool; NUM_THERMOCOUPLES],
    pub fan_enabled: [bool; NUM_FANS],
}

impl RunConfig {
    pub fn new(
        schedule: [InputScheduleStepValue; MAX_SCHEDULE_STEPS],
        enabled_tc_zones: [bool; NUM_THERMOCOUPLES],
        fan_enabled: [bool; NUM_FANS],
    ) -> Self {
        Self {
            schedule: core::array::from_fn(|i| ScheduleStep {
                duration: esp_hal::time::Duration::from_secs(schedule[i].duration as u64),
                temperature: schedule[i].temperature,
                ramp: schedule[i].ramp,
            }),
            enabled_tc_zones,
            fan_enabled,
        }
    }

    pub fn get_setpoint_for_time(&self, elapsed: esp_hal::time::Duration) -> f32 {
        let mut accumulated_time = esp_hal::time::Duration::from_secs(0);
        let mut previous_temperature = 0.0;
        for step in self.schedule.iter() {
            accumulated_time += step.duration;
            if elapsed < accumulated_time {
                if step.ramp {
                    let time_into_step =
                        (elapsed - (accumulated_time - step.duration)).as_millis() as f32;
                    let ramp_percent = time_into_step / step.duration.as_millis() as f32;
                    return previous_temperature
                        + ((step.temperature as f32 - previous_temperature) as f32 * ramp_percent);
                }
                return step.temperature as f32;
            }
            previous_temperature = step.temperature as f32;
        }
        0.0
    }

    pub fn get_total_run_time(&self) -> esp_hal::time::Duration {
        self.schedule
            .iter()
            .fold(esp_hal::time::Duration::from_secs(0), |acc, step| {
                acc + step.duration
            })
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
