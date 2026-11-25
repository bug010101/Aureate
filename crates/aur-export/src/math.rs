//! A pragmatic LaTeX → Typst math converter.
//!
//! Typst math is close enough to TeX that most formulas translate token by
//! token.  Commands we do not know are rendered as literal text instead of
//! being emitted verbatim: a single unknown macro would otherwise abort the
//! whole PDF compile, and a document with one odd symbol should still export.

use std::fmt::Write as _;

/// Convert a TeX fragment into Typst math source (without the `$` fences).
pub fn convert(tex: &str) -> String {
    let mut c = Conv {
        chars: tex.chars().collect(),
        i: 0,
    };
    let out = c.run(None);
    out.trim().to_string()
}

struct Conv {
    chars: Vec<char>,
    i: usize,
}

impl Conv {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.i).copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.i += 1;
        }
        c
    }

    fn skip_ws(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_whitespace()) {
            self.i += 1;
        }
    }

    fn run(&mut self, stop: Option<char>) -> String {
        let mut out = String::new();
        while let Some(ch) = self.peek() {
            if Some(ch) == stop {
                self.i += 1;
                break;
            }
            match ch {
                '\\' => {
                    self.i += 1;
                    self.command(&mut out);
                }
                '{' => {
                    self.i += 1;
                    let inner = self.run(Some('}'));
                    out.push_str(&inner);
                }
                '}' => {
                    // Unbalanced closing brace: drop it.
                    self.i += 1;
                }
                '^' | '_' => {
                    self.i += 1;
                    out.push(ch);
                    let (arg, atomic) = self.arg();
                    if atomic {
                        out.push_str(&arg);
                    } else {
                        let _ = write!(out, "({arg})");
                    }
                }
                '&' => {
                    self.i += 1;
                    out.push_str(" & ");
                }
                '~' => {
                    self.i += 1;
                    out.push(' ');
                }
                _ => {
                    self.i += 1;
                    out.push(ch);
                }
            }
        }
        out
    }

    /// Parse one argument: a braced group, a command, or a single character.
    /// Returns (converted, atomic).
    fn arg(&mut self) -> (String, bool) {
        self.skip_ws();
        match self.peek() {
            Some('{') => {
                self.i += 1;
                let inner = self.run(Some('}'));
                let atomic = inner.trim().chars().count() <= 1;
                (inner, atomic)
            }
            Some('\\') => {
                self.i += 1;
                let mut s = String::new();
                self.command(&mut s);
                (s, true)
            }
            Some(_) => {
                let ch = self.bump().unwrap();
                (ch.to_string(), true)
            }
            None => (String::new(), true),
        }
    }

    /// Optional `[...]` argument (e.g. `\sqrt[3]{x}`).
    fn opt_arg(&mut self) -> Option<String> {
        self.skip_ws();
        if self.peek() == Some('[') {
            self.i += 1;
            let mut depth = 1;
            let mut buf = String::new();
            while let Some(ch) = self.bump() {
                match ch {
                    '[' => {
                        depth += 1;
                        buf.push(ch);
                    }
                    ']' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                        buf.push(ch);
                    }
                    _ => buf.push(ch),
                }
            }
            let mut inner = Conv {
                chars: buf.chars().collect(),
                i: 0,
            };
            return Some(inner.run(None));
        }
        None
    }

    fn read_name(&mut self) -> String {
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_alphabetic() {
                name.push(c);
                self.i += 1;
            } else if c == '*' && !name.is_empty() {
                // \operatorname*{...}
                self.i += 1;
            } else {
                break;
            }
        }
        name
    }

    fn command(&mut self, out: &mut String) {
        let Some(first) = self.peek() else {
            out.push('\\');
            return;
        };

        // Escaped literals.
        if !first.is_ascii_alphabetic() {
            self.i += 1;
            match first {
                '\\' => out.push_str(" \\ "),
                ',' => out.push_str("thin"),
                ';' => out.push_str("med"),
                ':' => out.push_str("med"),
                '!' => {}
                ' ' => out.push(' '),
                '%' | '&' | '_' | '#' | '$' => out.push(first),
                '{' => out.push_str("\"{\""),
                '}' => out.push_str("\"}\""),
                '|' => out.push_str("bar.v"),
                _ => out.push(first),
            }
            return;
        }

        let name = self.read_name();

        match name.as_str() {
            // Structures with arguments.
            "frac" | "dfrac" | "tfrac" | "cfrac" => {
                let (a, _) = self.arg();
                let (b, _) = self.arg();
                let _ = write!(out, "frac({a}, {b})");
            }
            "binom" | "dbinom" | "tbinom" => {
                let (a, _) = self.arg();
                let (b, _) = self.arg();
                let _ = write!(out, "binom({a}, {b})");
            }
            "sqrt" => {
                if let Some(n) = self.opt_arg() {
                    let (a, _) = self.arg();
                    let _ = write!(out, "root({n}, {a})");
                } else {
                    let (a, _) = self.arg();
                    let _ = write!(out, "sqrt({a})");
                }
            }
            "text" | "textrm" | "mbox" | "hbox" | "mathrm" | "operatorname" | "textnormal" => {
                let (a, _) = self.arg();
                let _ = write!(out, "\"{}\"", escape_str(&a));
            }
            "mathbf" | "boldsymbol" | "bm" | "pmb" => {
                let (a, _) = self.arg();
                let _ = write!(out, "bold({a})");
            }
            "mathit" | "textit" | "emph" => {
                let (a, _) = self.arg();
                let _ = write!(out, "italic({a})");
            }
            "mathbb" | "Bbb" => {
                let (a, _) = self.arg();
                let _ = write!(out, "bb({a})");
            }
            "mathcal" | "mathscr" | "pazocal" => {
                let (a, _) = self.arg();
                let _ = write!(out, "cal({a})");
            }
            "mathfrak" => {
                let (a, _) = self.arg();
                let _ = write!(out, "frak({a})");
            }
            "mathsf" => {
                let (a, _) = self.arg();
                let _ = write!(out, "sans({a})");
            }
            "mathtt" | "texttt" => {
                let (a, _) = self.arg();
                let _ = write!(out, "mono({a})");
            }
            "overline" | "bar" => {
                let (a, _) = self.arg();
                let _ = write!(out, "overline({a})");
            }
            "underline" => {
                let (a, _) = self.arg();
                let _ = write!(out, "underline({a})");
            }
            "hat" | "widehat" => {
                let (a, _) = self.arg();
                let _ = write!(out, "hat({a})");
            }
            "tilde" | "widetilde" => {
                let (a, _) = self.arg();
                let _ = write!(out, "tilde({a})");
            }
            "vec" => {
                let (a, _) = self.arg();
                let _ = write!(out, "accent({a}, arrow.r)");
            }
            "dot" => {
                let (a, _) = self.arg();
                let _ = write!(out, "dot({a})");
            }
            "ddot" => {
                let (a, _) = self.arg();
                let _ = write!(out, "dot.double({a})");
            }
            "overbrace" => {
                let (a, _) = self.arg();
                let _ = write!(out, "overbrace({a})");
            }
            "underbrace" => {
                let (a, _) = self.arg();
                let _ = write!(out, "underbrace({a})");
            }
            "overset" => {
                let (a, _) = self.arg();
                let (b, _) = self.arg();
                let _ = write!(out, "attach({b}, t: {a})");
            }
            "underset" => {
                let (a, _) = self.arg();
                let (b, _) = self.arg();
                let _ = write!(out, "attach({b}, b: {a})");
            }
            "stackrel" => {
                let (a, _) = self.arg();
                let (b, _) = self.arg();
                let _ = write!(out, "attach({b}, t: {a})");
            }
            "substack" => {
                let (a, _) = self.arg();
                let _ = write!(out, "({a})");
            }
            "begin" => {
                let (env, _) = self.arg();
                self.environment(&env, out);
            }
            "end" => {
                let _ = self.arg();
            }
            "left" | "right" | "big" | "Big" | "bigg" | "Bigg" | "bigl" | "bigr" | "Bigl"
            | "Bigr" | "biggl" | "biggr" | "middle" => {
                // The delimiter itself follows as a normal character.
                self.skip_ws();
                if self.peek() == Some('.') {
                    self.i += 1;
                }
            }
            "quad" => out.push_str("wide"),
            "qquad" => out.push_str("wide wide"),
            "limits" | "nolimits" | "displaystyle" | "textstyle" | "scriptstyle" | "nonumber"
            | "notag" | "label" | "tag" | "hspace" | "vspace" | "phantom" | "hphantom"
            | "vphantom" => {
                if matches!(name.as_str(), "hspace" | "vspace" | "label" | "tag") {
                    let _ = self.arg();
                }
            }
            _ => {
                if let Some(sym) = symbol(&name) {
                    push_symbol(out, sym);
                } else {
                    // Unknown macro: show it verbatim rather than break the
                    // compile.
                    let _ = write!(out, "\"\\\\{name}\"");
                }
            }
        }
    }

    fn environment(&mut self, env: &str, out: &mut String) {
        // Collect the body up to \end{env}.
        let mut body = String::new();
        let mut depth = 1;
        while self.i < self.chars.len() {
            if self.chars[self.i] == '\\' {
                let rest: String = self.chars[self.i..].iter().take(6).collect();
                if rest.starts_with("\\begin") {
                    depth += 1;
                } else if rest.starts_with("\\end") {
                    depth -= 1;
                    if depth == 0 {
                        // Consume \end{...}
                        self.i += 4;
                        let _ = self.arg();
                        break;
                    }
                }
                body.push('\\');
                self.i += 1;
                if let Some(c) = self.bump() {
                    body.push(c);
                }
            } else {
                body.push(self.chars[self.i]);
                self.i += 1;
            }
        }

        let env_clean = env.trim().trim_start_matches('{').trim_end_matches('}');
        let rows = split_rows(&body);
        let matrix = matches!(
            env_clean,
            "matrix"
                | "smallmatrix"
                | "pmatrix"
                | "bmatrix"
                | "Bmatrix"
                | "vmatrix"
                | "Vmatrix"
                | "array"
                | "cases"
        );

        if matrix {
            let converted_rows: Vec<String> = rows
                .iter()
                .map(|row| {
                    row.split('&')
                        .map(|cell| {
                            let mut c = Conv {
                                chars: cell.chars().collect(),
                                i: 0,
                            };
                            c.run(None).trim().to_string()
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .collect();

            let body = converted_rows.join("; ");
            match env_clean {
                "pmatrix" => {
                    let _ = write!(out, "mat(delim: \"(\", {body})");
                }
                "bmatrix" => {
                    let _ = write!(out, "mat(delim: \"[\", {body})");
                }
                "Bmatrix" => {
                    let _ = write!(out, "mat(delim: \"{{\", {body})");
                }
                "vmatrix" => {
                    let _ = write!(out, "mat(delim: \"|\", {body})");
                }
                "Vmatrix" => {
                    let _ = write!(out, "mat(delim: \"||\", {body})");
                }
                "cases" => {
                    let _ = write!(out, "cases({body})");
                }
                _ => {
                    let _ = write!(out, "mat({body})");
                }
            }
        } else {
            // aligned / gather / equation / split … keep rows with alignment.
            let converted: Vec<String> = rows
                .iter()
                .map(|row| {
                    let mut c = Conv {
                        chars: row.chars().collect(),
                        i: 0,
                    };
                    c.run(None).trim().to_string()
                })
                .collect();
            out.push_str(&converted.join(" \\ "));
        }
    }
}

fn split_rows(body: &str) -> Vec<String> {
    let mut rows = Vec::new();
    let mut current = String::new();
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            if chars.peek() == Some(&'\\') {
                chars.next();
                rows.push(std::mem::take(&mut current));
                continue;
            }
            current.push('\\');
            if let Some(next) = chars.next() {
                current.push(next);
            }
        } else {
            current.push(ch);
        }
    }
    if !current.trim().is_empty() {
        rows.push(current);
    }
    rows
}

fn escape_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Append a symbol name, keeping a word boundary against what came before.
///
/// Without this `i\pi` would become `ipi`, which Typst reads as one unknown
/// identifier — a silent, very confusing error inside a formula.
fn push_symbol(out: &mut String, sym: &str) {
    if out
        .chars()
        .last()
        .is_some_and(|c| c.is_alphanumeric() || c == '.')
    {
        out.push(' ');
    }
    out.push_str(sym);
}

fn symbol(name: &str) -> Option<&'static str> {
    Some(match name {
        // Greek
        "alpha" => "alpha",
        "beta" => "beta",
        "gamma" => "gamma",
        "delta" => "delta",
        "epsilon" => "epsilon",
        "varepsilon" => "epsilon.alt",
        "zeta" => "zeta",
        "eta" => "eta",
        "theta" => "theta",
        "vartheta" => "theta.alt",
        "iota" => "iota",
        "kappa" => "kappa",
        "lambda" => "lambda",
        "mu" => "mu",
        "nu" => "nu",
        "xi" => "xi",
        "pi" => "pi",
        "varpi" => "pi.alt",
        "rho" => "rho",
        "varrho" => "rho.alt",
        "sigma" => "sigma",
        "varsigma" => "sigma.alt",
        "tau" => "tau",
        "upsilon" => "upsilon",
        "phi" => "phi",
        "varphi" => "phi.alt",
        "chi" => "chi",
        "psi" => "psi",
        "omega" => "omega",
        "Gamma" => "Gamma",
        "Delta" => "Delta",
        "Theta" => "Theta",
        "Lambda" => "Lambda",
        "Xi" => "Xi",
        "Pi" => "Pi",
        "Sigma" => "Sigma",
        "Upsilon" => "Upsilon",
        "Phi" => "Phi",
        "Psi" => "Psi",
        "Omega" => "Omega",
        // Big operators
        "sum" => "sum",
        "prod" => "product",
        "coprod" => "product.co",
        "int" => "integral",
        "iint" => "integral.double",
        "iiint" => "integral.triple",
        "oint" => "integral.cont",
        "bigcup" => "union.big",
        "bigcap" => "sect.big",
        "bigoplus" => "plus.circle.big",
        "bigotimes" => "times.circle.big",
        // Relations
        "le" | "leq" => "<=",
        "ge" | "geq" => ">=",
        "ne" | "neq" => "!=",
        "approx" => "approx",
        "equiv" => "equiv",
        "sim" => "tilde.op",
        "simeq" => "tilde.eq",
        "cong" => "tilde.equiv",
        "propto" => "prop",
        "ll" => "lt.double",
        "gg" => "gt.double",
        "prec" => "prec",
        "succ" => "succ",
        "perp" => "perp",
        "parallel" => "parallel",
        "in" => "in",
        "notin" => "in.not",
        "ni" => "in.rev",
        "subset" => "subset",
        "subseteq" => "subset.eq",
        "supset" => "supset",
        "supseteq" => "supset.eq",
        "cup" => "union",
        "cap" => "sect",
        "setminus" => "without",
        "emptyset" | "varnothing" => "emptyset",
        "mid" => "bar.v",
        "vdash" => "tack.r",
        "models" => "models",
        // Arrows
        "to" | "rightarrow" => "arrow.r",
        "leftarrow" | "gets" => "arrow.l",
        "leftrightarrow" => "arrow.l.r",
        "Rightarrow" | "implies" => "arrow.r.double",
        "Leftarrow" => "arrow.l.double",
        "Leftrightarrow" => "arrow.l.r.double",
        "mapsto" => "arrow.r.bar",
        "uparrow" => "arrow.t",
        "downarrow" => "arrow.b",
        "longrightarrow" => "arrow.r.long",
        "longleftarrow" => "arrow.l.long",
        "rightleftharpoons" => "harpoons.r",
        // Operators & misc
        "pm" => "plus.minus",
        "mp" => "minus.plus",
        "times" => "times",
        "div" => "div",
        "cdot" => "dot.op",
        "ast" => "ast.op",
        "star" => "star",
        "circ" => "circle.stroked.small",
        "bullet" => "bullet",
        "oplus" => "plus.circle",
        "otimes" => "times.circle",
        "infty" => "infinity",
        "partial" => "diff",
        "nabla" => "nabla",
        "forall" => "forall",
        "exists" => "exists",
        "nexists" => "exists.not",
        "neg" | "lnot" => "not",
        "land" | "wedge" => "and",
        "lor" | "vee" => "or",
        "angle" => "angle",
        "measuredangle" => "angle.measured",
        "triangle" => "triangle",
        "square" => "square",
        "degree" => "degree",
        "prime" => "prime",
        "ldots" | "dots" => "dots.h",
        "cdots" => "dots.h.c",
        "vdots" => "dots.v",
        "ddots" => "dots.down",
        "hbar" => "planck.reduce",
        "ell" => "ell",
        "Re" => "Re",
        "Im" => "Im",
        "aleph" => "aleph",
        "therefore" => "therefore",
        "because" => "because",
        "checkmark" => "checkmark",
        // Functions rendered upright
        "sin" => "sin",
        "cos" => "cos",
        "tan" => "tan",
        "cot" => "cot",
        "sec" => "sec",
        "csc" => "csc",
        "arcsin" => "arcsin",
        "arccos" => "arccos",
        "arctan" => "arctan",
        "sinh" => "sinh",
        "cosh" => "cosh",
        "tanh" => "tanh",
        "log" => "log",
        "ln" => "ln",
        "lg" => "lg",
        "exp" => "exp",
        "lim" => "lim",
        "limsup" => "limsup",
        "liminf" => "liminf",
        "sup" => "sup",
        "inf" => "inf",
        "max" => "max",
        "min" => "min",
        "arg" => "arg",
        "det" => "det",
        "dim" => "dim",
        "ker" => "ker",
        "deg" => "deg",
        "gcd" => "gcd",
        "hom" => "hom",
        "mod" => "mod",
        "bmod" => "mod",
        "quad_placeholder" => "",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_fraction_and_power() {
        assert_eq!(convert(r"\frac{a}{b}"), "frac(a, b)");
        assert_eq!(convert(r"x^{2}"), "x^2");
        assert_eq!(convert(r"x^{n+1}"), "x^(n+1)");
        assert_eq!(convert(r"e^{i\pi}"), "e^(i pi)");
    }

    #[test]
    fn greek_and_relations() {
        assert_eq!(convert(r"\alpha \le \beta"), "alpha <= beta");
        assert_eq!(convert(r"\sum_{i=1}^{n} i"), "sum_(i=1)^n i");
    }

    #[test]
    fn sqrt_and_text() {
        assert_eq!(convert(r"\sqrt{x^2+1}"), "sqrt(x^2+1)");
        assert_eq!(convert(r"\sqrt[3]{x}"), "root(3, x)");
        assert_eq!(convert(r"\text{if } x"), "\"if \" x");
    }

    #[test]
    fn matrix_and_cases() {
        let m = convert(r"\begin{pmatrix} a & b \\ c & d \end{pmatrix}");
        assert_eq!(m, "mat(delim: \"(\", a, b; c, d)");
        let c = convert(r"\begin{cases} 1 & x > 0 \\ 0 & \text{else} \end{cases}");
        assert!(c.starts_with("cases("), "{c}");
        assert!(c.contains(";"), "{c}");
    }

    #[test]
    fn unknown_command_is_literal() {
        let s = convert(r"\weirdcmd x");
        assert!(s.contains("\"\\\\weirdcmd\""), "{s}");
    }
}
