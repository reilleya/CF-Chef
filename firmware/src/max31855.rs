use bitfield::Bit;
use esp_println::println;

pub enum MAX31855Reading {
    Valid {
        temp: f32,
        internal_temp: f32,
    },
    Fault {
        open_circuit: bool,
        short_to_vcc: bool,
        short_to_gnd: bool,
    },
}

pub fn interpret_max31855_read(buffer: [u8; 4]) -> MAX31855Reading {
    let fault = buffer[1].bit(0);
    if fault {
        return MAX31855Reading::Fault {
            open_circuit: buffer[1].bit(2),
            short_to_vcc: buffer[1].bit(1),
            short_to_gnd: buffer[1].bit(0),
        };
    }

    // TODO: does this slop handle negative temperatures?
    // TODO: use bitfields

    let bits: u32 = (buffer[0] as u32) << 24
        | (buffer[1] as u32) << 16
        | (buffer[2] as u32) << 8
        | (buffer[3] as u32) << 0;
    let temp = ((bits >> 18) & 0x3FFF) as i16; // Extract bits 31-18 and interpret as signed
    let temp = temp as f32 * 0.25; // Each bit represents 0.25 degrees Celsius

    let internal_temp = ((bits >> 4) & 0xFFF) as i16; // Extract bits 15-4 and interpret as signed
    let internal_temp = internal_temp as f32 * 0.0625; // Each bit represents 0.0625 degrees Celsius

    MAX31855Reading::Valid {
        temp,
        internal_temp,
    }
}

pub fn log_max31855_reading(reading: &MAX31855Reading) {
    match reading {
        MAX31855Reading::Valid {
            temp,
            internal_temp,
        } => {
            println!("Temperature: {temp:.2} °C, Internal Temperature: {internal_temp:.2} °C");
        }
        MAX31855Reading::Fault {
            open_circuit,
            short_to_vcc,
            short_to_gnd,
        } => {
            println!(
                "Fault detected! Open Circuit: {open_circuit}, Short to VCC: {short_to_vcc}, Short to GND: {short_to_gnd}"
            );
        }
    }
}
