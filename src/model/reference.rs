//! ---- feat/reference-clips ----
//! Where a reference clip came from (Find a Reference, `ui::app::reference_ctl`): the source page, the
//! site, the part that was downloaded, and the words said in it once transcribed - so it can be found by
//! quote in the Library and credited with "Copy credits". Ops: `model::ops::references`.

use crate::model::Id;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RefSource {
    /// The library asset this describes.
    pub asset: Id,
    /// "YouTube", "GIPHY", "Tenor", "Yarn", a host name...
    pub site: String,
    /// The page the user picked it from (what the credits link to).
    pub url: String,
    pub title: String,
    /// The uploader / studio, when known.
    pub credit: String,
    /// The part downloaded, in the source's seconds ("Just a part"); None = the whole thing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<(f64, f64)>,
    /// What is said in it (filled by the automatic transcription; editable).
    pub quote: String,
}

/// The library tag every reference carries: the References smart bin and a plain search both find it.
pub const REFERENCE_TAG: &str = "reference";
/// The smart bin made the first time a reference is imported.
pub const REFERENCES_BIN: &str = "References";
