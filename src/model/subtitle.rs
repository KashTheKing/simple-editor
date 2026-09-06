use crate::model::*;
use serde::{Deserialize, Serialize};

// ---------- subtitles ----------

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Cue {
    pub id: Id,
    pub start: f64,
    pub end: f64,
    pub text: String,
}
