//! Export options shared between the UI and the Typst generator.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    Github,
    Academic,
    Business,
    Minimal,
}

impl Theme {
    pub fn label(&self) -> &'static str {
        match self {
            Theme::Github => "GitHub",
            Theme::Academic => "学术论文",
            Theme::Business => "商务报告",
            Theme::Minimal => "极简",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Paper {
    #[default]
    A4,
    A5,
    A6,
    Letter,
    Legal,
}

impl Paper {
    /// Paper name accepted by Typst's `page` function.
    pub fn typst_name(&self) -> &'static str {
        match self {
            Paper::A4 => "a4",
            Paper::A5 => "a5",
            Paper::A6 => "a6",
            Paper::Letter => "us-letter",
            Paper::Legal => "us-legal",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Paper::A4 => "A4",
            Paper::A5 => "A5",
            Paper::A6 => "A6",
            Paper::Letter => "Letter",
            Paper::Legal => "Legal",
        }
    }

    /// Width / height in millimetres, used by the preview to size the sheet.
    pub fn size_mm(&self) -> (f32, f32) {
        match self {
            Paper::A4 => (210.0, 297.0),
            Paper::A5 => (148.0, 210.0),
            Paper::A6 => (105.0, 148.0),
            Paper::Letter => (215.9, 279.4),
            Paper::Legal => (215.9, 355.6),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Margin {
    Narrow,
    #[default]
    Normal,
    Wide,
}

impl Margin {
    /// (horizontal, vertical) in centimetres.
    pub fn cm(&self) -> (f32, f32) {
        match self {
            Margin::Narrow => (1.6, 1.8),
            Margin::Normal => (2.2, 2.4),
            Margin::Wide => (3.0, 3.0),
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Margin::Narrow => "窄",
            Margin::Normal => "标准",
            Margin::Wide => "宽",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExportOptions {
    pub theme: Theme,
    pub paper: Paper,
    pub landscape: bool,
    pub margin: Margin,
    /// Header template; supports `{title}` and `{date}`.
    pub header: String,
    pub footer_page_numbers: bool,
    pub toc: bool,
    pub toc_own_page: bool,
    pub h1_page_break: bool,
    pub font_size: f32,
    pub line_height: f32,
    pub first_line_indent: bool,
    /// Used to fill `{date}`; today when empty.
    pub date: String,
    /// Overrides the document title when non-empty.
    pub title: String,
    /// Render formulas as literal TeX when the math conversion fails.
    pub math_as_text: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        ExportOptions {
            theme: Theme::Github,
            paper: Paper::A4,
            landscape: false,
            margin: Margin::Normal,
            header: String::new(),
            footer_page_numbers: true,
            toc: false,
            toc_own_page: false,
            h1_page_break: false,
            font_size: 11.0,
            line_height: 0.78,
            first_line_indent: false,
            date: String::new(),
            title: String::new(),
            math_as_text: false,
        }
    }
}

impl ExportOptions {
    /// Theme-appropriate starting point: academic layout indents paragraphs
    /// and puts every level-1 heading on a new page.
    pub fn for_theme(theme: Theme) -> Self {
        let mut o = ExportOptions {
            theme,
            ..Default::default()
        };
        if theme == Theme::Academic {
            o.first_line_indent = true;
            o.line_height = 0.85;
            o.h1_page_break = true;
            o.toc = true;
            o.toc_own_page = true;
        }
        if theme == Theme::Business {
            o.toc = true;
        }
        o
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CardPage {
    #[default]
    A6,
    A5,
}

impl CardPage {
    pub fn typst_name(&self) -> &'static str {
        match self {
            CardPage::A6 => "a6",
            CardPage::A5 => "a5",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CardbookOptions {
    pub theme: Theme,
    pub page: CardPage,
    pub header: String,
    pub footer_page_numbers: bool,
    /// Include the section headings (`## 问题` …) in the card page.
    pub show_sections: bool,
    pub font_size: f32,
}

impl Default for CardbookOptions {
    fn default() -> Self {
        CardbookOptions {
            theme: Theme::Academic,
            page: CardPage::A6,
            header: String::new(),
            footer_page_numbers: true,
            show_sections: true,
            font_size: 9.0,
        }
    }
}
