//! `semtag-core`: the note-side half of semtag.
//!
//! Everything here is pure text in, text out: parsing and writing the hidden tag block, the inline
//! labels block, the TSNS structure block, front-matter counts, Mermaid, the theory CSV, and fold
//! toggling. No network I/O lives in this crate.

pub mod csv_export;
pub mod fold;
pub mod frontmatter;
pub mod inline;
pub mod item;
pub mod mermaid;
pub mod normalize;
pub mod note;
pub mod ops;
pub mod tag;
pub mod tsns;
pub mod uri;

pub use fold::{Block, Fold, FoldConfig};
pub use item::{items_to_tags, ModelItem};
pub use note::{
    apply, strip_blocks, strip_for_model, StrictFailure, TsnsMode, WriteOptions, WriteOutcome,
};
pub use ops::Operation;
pub use tag::{parse_tags, Tag};
