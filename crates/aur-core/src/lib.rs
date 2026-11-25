//! `aur-core` — document and card domain model.
//!
//! This crate is the bottom of the dependency graph: it knows markdown,
//! frontmatter and cards, and nothing about Tauri, SQLite, Typst or the UI.

pub mod ast;
pub mod card;
pub mod document;
pub mod frontmatter;
pub mod markdown;

pub use ast::{first_heading, plain_all, walk_blocks, wiki_links, Align, Block, Inline, ListItem};
pub use card::{
    join_sections, new_card_markdown, sections_from_body, split_sections, week_start, Card,
    CardKind, CardSection, CardStatus,
};
pub use document::{title_from_path, word_count, Document};
pub use frontmatter::Frontmatter;
