use serde::{Deserialize, Serialize};

/// One display band with an upper bound and optional text/colour. Bands are
/// sorted by `upper`; `upper = None` is the final open-ended band. `inclusive`
/// selects `value <= upper` versus `value < upper`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Band {
    #[serde(default)]
    pub upper: Option<f64>,
    #[serde(default)]
    pub inclusive: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u8>,
}

impl Band {
    pub fn final_band(color: Option<u8>) -> Self {
        Self {
            upper: None,
            inclusive: false,
            text: String::new(),
            color,
        }
    }

    pub fn matches(&self, value: f64) -> bool {
        match self.upper {
            None => true,
            Some(upper) if self.inclusive => value <= upper,
            Some(upper) => value < upper,
        }
    }
}

/// First band matching `value`.
pub fn band_index(bands: &[Band], value: f64) -> Option<usize> {
    bands.iter().position(|band| band.matches(value))
}

/// Convert the legacy cut/colour pair into bands. A cut is the start of the
/// next band, so every cut becomes a `< cut` band and the last band is open.
pub fn from_cuts(cuts: &[f64], colors: &[usize], names: &[String]) -> Vec<Band> {
    let count = cuts.len() + 1;
    let mut bands: Vec<Band> = (0..count)
        .map(|index| {
            let upper = cuts.get(index).copied();
            Band {
                upper,
                inclusive: false,
                text: names.get(index).cloned().unwrap_or_default(),
                color: colors.get(index).copied().map(|slot| slot as u8),
            }
        })
        .collect();
    if bands.is_empty() {
        bands.push(Band::final_band(None));
    }
    bands
}

/// A reusable, project-owned set of value bands. Targets reference `id`; the
/// set itself has no owner, so several rows/controls can share it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BandSet {
    pub id: String,
    pub name: String,
    pub bands: Vec<Band>,
}
