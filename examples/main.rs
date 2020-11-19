use rustsynth::{
    host::Host,
    midi::MidiEvents,
    midi::MidiInput,
    midi::MidiSlider,
    midi::MidiSliderSettings,
    modules::Envelope,
    modules::EnvelopeSettings,
    modules::{Op, OpType, Oscillator, OscillatorSettings},
};

use std::error::Error;

fn main() {
    match run() {
        Ok(_) => (),
        Err(err) => println!("Error: {}", err),
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut host = Host::new();

    let midi = host.create_module::<MidiInput>(0);

    let fmod_pitch_slider = host.create_module::<MidiSlider>(MidiSliderSettings {
        controller: 41,
        default: 1.0,
        min: 0.0,
        max: 4.0,
    });
    host.link::<MidiEvents>(midi, "out", fmod_pitch_slider, "in");

    let fmod_vol_slider = host.create_module::<MidiSlider>(MidiSliderSettings {
        controller: 42,
        default: 64.0,
        min: 0.0,
        max: 128.0,
    });
    host.link::<MidiEvents>(midi, "out", fmod_vol_slider, "in");

    let fmod_osc = host.create_module::<Oscillator>(OscillatorSettings::Square);
    host.link::<MidiEvents>(midi, "out", fmod_osc, "in");
    host.link::<f32>(fmod_pitch_slider, "out", fmod_osc, "pitch_shift");

    let fmod_envelope = host.create_module::<Envelope>(EnvelopeSettings {
        attack: 0.0,
        decay: 5.0,
        sustain: 0.6,
        release: 0.2,
    });
    host.link::<MidiEvents>(midi, "out", fmod_envelope, "in");
    host.link::<f32>(fmod_osc, "out", fmod_envelope, "in");

    let fmod_amp = host.create_module::<Op>(OpType::Multiply(2));
    host.link::<f32>(fmod_envelope, "out", fmod_amp, "0");
    host.link::<f32>(fmod_vol_slider, "out", fmod_amp, "1");

    let carrier_osc = host.create_module::<Oscillator>(OscillatorSettings::Sine(1024));
    host.link::<MidiEvents>(midi, "out", carrier_osc, "in");
    host.link_value::<f32>(0.2, carrier_osc, "vel_amt");
    host.link::<f32>(fmod_amp, "out", carrier_osc, "freq_mod");

    let carrier_envelope = host.create_module::<Envelope>(EnvelopeSettings {
        attack: 0.0,
        decay: 1.0,
        sustain: 0.6,
        release: 0.6,
    });
    host.link::<MidiEvents>(midi, "out", carrier_envelope, "in");
    host.link::<f32>(carrier_osc, "out", carrier_envelope, "in");

    let output = host.output_module();
    host.link::<f32>(carrier_envelope, "out", output, "in");

    host.process();
}
