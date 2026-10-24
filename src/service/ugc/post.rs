//! How posts look where something on a post shows it. Posts can't be reported, so there is no
//! adapter here.

use super::{LegacyCard, Preview, UgcPreview};
use crate::db::post::Post;
use serde::{Deserialize, Serialize};

/// @since 261024
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PostPreview {
    pub id: i64,
    pub title: String,
    pub cover_url: Option<String>,
}

impl Preview for PostPreview {
    fn headline(&self) -> &str {
        &self.title
    }

    fn legacy_card(&self) -> LegacyCard {
        LegacyCard { title: self.title.clone(), cover_url: self.cover_url.clone(), display_id: String::new() }
    }
}

pub fn preview(x: &Post) -> UgcPreview {
    UgcPreview::Post(PostPreview { id: x.id, title: x.title.clone(), cover_url: x.cover_url.clone() })
}
