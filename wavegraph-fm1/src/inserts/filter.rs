//! Retrigger a strip's filter on the same sixteen cells shown by the Beat FX playhead.

use crate::fx::FilterSettings;
use wavegraph_fx::{FILTER_PATTERNS, PatternFilter, Slicer};
use wavegraph_lfo::Phase;
use wavegraph_sample::I24;

#[derive(Clone, Copy)]
pub(super) struct ClockedFilter {
    dsp: PatternFilter,
    settings: FilterSettings,
    last_step: Option<u8>,
}

impl ClockedFilter {
    pub(super) fn new(rate_hz: u32) -> Self {
        let settings = FilterSettings::INITIAL;
        let mut dsp = PatternFilter::new(rate_hz).expect("audio sample rate");
        dsp.set_controls(settings.controls());
        Self {
            dsp,
            settings,
            last_step: None,
        }
    }

    pub(super) fn configure(&mut self, settings: FilterSettings) {
        let controls = settings.controls();
        let mut previous = self.settings.controls();
        previous.enabled = controls.enabled;
        if controls != previous {
            self.dsp.set_controls(controls);
        } else {
            self.dsp.set_enabled(controls.enabled);
        }
        if self.settings.pick.rhythm != settings.pick.rhythm || !self.settings.pick.enabled {
            self.last_step = None;
        }
        self.settings = settings;
    }

    pub(super) fn process(&mut self, samples: &mut [I24], mut phase: Phase, restart: bool) {
        if !self.settings.pick.enabled {
            self.dsp.bypass_in_place(samples);
            return;
        }
        if restart {
            self.last_step = None;
        }
        let pattern = &FILTER_PATTERNS[usize::from(self.settings.pick.rhythm)];
        for sample in samples {
            let step = Slicer::step_at(phase.position());
            if self.last_step != Some(step) {
                self.dsp.trigger(pattern.accents[usize::from(step)]);
                self.last_step = Some(step);
            }
            *sample = self.dsp.tick(*sample);
            phase.advance(1);
        }
    }
}
