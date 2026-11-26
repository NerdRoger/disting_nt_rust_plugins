pub(crate) trait LayerModel: Copy + PartialEq {
    const OFF: Self;
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct LayerPatch<Model, const CONTROLS: usize> {
    pub(crate) model: Model,
    pub(crate) controls: [f32; CONTROLS],
}

impl<Model: LayerModel, const CONTROLS: usize> LayerPatch<Model, CONTROLS> {
    pub(crate) const OFF: Self = Self {
        model: Model::OFF,
        controls: [0.0; CONTROLS],
    };
}

#[derive(Clone, Copy)]
pub(crate) struct Layer<Model, const CONTROLS: usize> {
    pub(crate) patch: LayerPatch<Model, CONTROLS>,
    pub(crate) level: f32,
    pub(crate) solo: bool,
    pub(crate) mute: bool,
}

impl<Model: LayerModel, const CONTROLS: usize> Layer<Model, CONTROLS> {
    pub(crate) const OFF: Self = Self {
        patch: LayerPatch::OFF,
        level: 0.0,
        solo: false,
        mute: false,
    };

    pub(crate) fn solo_active(layers: &[Self]) -> bool {
        // A soloed model still excludes other layers when muted or at zero level.
        layers
            .iter()
            .any(|layer| layer.patch.model != Model::OFF && layer.solo)
    }

    pub(crate) fn audible(&self, solo_active: bool) -> bool {
        self.patch.model != Model::OFF
            && !self.mute
            && (!solo_active || self.solo)
            && self.level != 0.0
    }
}

#[cfg(test)]
mod tests {
    use super::{Layer, LayerModel, LayerPatch};

    #[derive(Clone, Copy, PartialEq)]
    enum Model {
        Off,
        On,
    }

    impl LayerModel for Model {
        const OFF: Self = Self::Off;
    }

    #[test]
    fn configured_layers_engage_solo_even_if_inaudible() {
        let audible = Layer {
            patch: LayerPatch {
                model: Model::On,
                controls: [0.0; 1],
            },
            level: 1.0,
            ..Layer::OFF
        };
        let mut soloed = Layer {
            solo: true,
            ..Layer::OFF
        };
        assert!(!Layer::solo_active(&[audible, soloed]));
        assert!(audible.audible(false));

        soloed.patch.model = Model::On;
        let solo_active = Layer::solo_active(&[audible, soloed]);
        assert!(solo_active);
        assert!(!audible.audible(solo_active));
        assert!(!soloed.audible(solo_active));

        soloed.level = 1.0;
        soloed.mute = true;
        assert!(Layer::solo_active(&[audible, soloed]));
        assert!(!soloed.audible(true));
    }
}
