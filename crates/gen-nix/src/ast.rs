//! Typed Nix expression AST. Implements `theory/NIX-AST.md` — every
//! Nix construct emitted by pleme-io renderers maps to one variant.
//! `format!()` of nix syntax is the antipattern this AST replaces.

/// Comprehensive Nix expression value. Atoms + identifiers +
/// collections + lambdas + let/with/if + operators + an escape hatch
/// for the not-yet-typed.
#[derive(Clone, Debug, PartialEq)]
pub enum NixValue {
    // Atoms
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    /// Multi-line indented string (`'' ... ''`).
    IndentedStr(Vec<String>),
    /// Path literal — `./foo`, `/abs/path`, `<nixpkgs>`. Renderer does
    /// not quote.
    Path(String),
    /// String with `${...}` interpolation.
    InterpolatedStr(Vec<StrPart>),

    // Identifiers + access
    Ident(String),
    /// Dotted access: `pkgs.lib.eachDefaultSystem`.
    AttrPath(Vec<String>),

    // Collections
    List(Vec<NixValue>),
    AttrSet {
        recursive: bool,
        entries: Vec<AttrSetEntry>,
    },

    // Functions
    Lambda {
        params: LambdaParams,
        body: Box<NixValue>,
    },
    Apply {
        func: Box<NixValue>,
        args: Vec<NixValue>,
    },

    // Control / scoping
    Let {
        bindings: Vec<LetBinding>,
        body: Box<NixValue>,
    },
    With {
        scope: Box<NixValue>,
        body: Box<NixValue>,
    },
    If {
        cond: Box<NixValue>,
        then_branch: Box<NixValue>,
        else_branch: Box<NixValue>,
    },

    // Operators
    BinOp {
        op: NixBinOp,
        left: Box<NixValue>,
        right: Box<NixValue>,
    },
    UnaryOp {
        op: NixUnaryOp,
        operand: Box<NixValue>,
    },
    AttrOr {
        attrset: Box<NixValue>,
        attr: Vec<String>,
        default: Box<NixValue>,
    },
    HasAttr {
        attrset: Box<NixValue>,
        attr: Vec<String>,
    },

    /// Indented string (`'' ... ''`) with `${...}` interpolation, one
    /// inner `Vec` per line. Literal parts are escaped for the indented
    /// form (`''` becomes `'''`, `${` becomes `''${`), unlike
    /// [`Self::IndentedStr`], which is verbatim.
    IndentedInterpolatedStr(Vec<Vec<StrPart>>),

    /// Attribute selection over a path that may interpolate, with an
    /// optional `or` default: `sources.${name}.meta.version`,
    /// `sources.${system} or (throw "...")`. A non-atomic default is
    /// parenthesized, so `x.a or (f y)` never parses as `(x.a or f) y`.
    Select {
        base: Box<NixValue>,
        path: AttrPath,
        default: Option<Box<NixValue>>,
    },

    /// Leading `#` comment lines on an expression: a file header, or a
    /// note above a nested value. Each line renders as `# <line>` at the
    /// current indent (`#` alone for an empty line).
    Commented {
        comments: Vec<String>,
        value: Box<NixValue>,
    },

    // Layout siblings: the same semantics as the canonical constructs
    // above, with the layout chosen by the caller instead of by the
    // renderer's heuristic. The canonical variants keep their output.
    /// Attrset with an explicit [`CollectionLayout`].
    LaidOutAttrSet {
        recursive: bool,
        entries: Vec<AttrSetEntry>,
        layout: CollectionLayout,
    },
    /// List with an explicit [`CollectionLayout`].
    LaidOutList {
        items: Vec<NixValue>,
        layout: CollectionLayout,
    },
    /// `let ... in` with an explicit [`LetLayout`].
    LaidOutLet {
        bindings: Vec<LetBinding>,
        body: Box<NixValue>,
        layout: LetLayout,
    },
    /// Lambda with an explicit [`LambdaLayout`].
    LaidOutLambda {
        params: LambdaParams,
        body: Box<NixValue>,
        layout: LambdaLayout,
    },

    /// Verbatim Nix. Every Raw call site is a debt against the AST —
    /// promote to a typed variant when the shape becomes recurrent.
    Raw(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum AttrSetEntry {
    KeyValue {
        key: AttrPath,
        value: NixValue,
    },
    Inherit {
        from: Option<NixValue>,
        names: Vec<String>,
    },
    /// A `# <text>` line of its own. Inside an inline attrset it renders
    /// as `/* <text> */`, so it can never swallow the rest of the line.
    Comment(String),
    /// An empty line between entries (no indentation). Dropped inline.
    Blank,
    /// Several entries on one line, with an optional trailing comment:
    /// `owner = "x"; repo = "y"; # fork: ...`.
    Line {
        entries: Vec<AttrSetEntry>,
        comment: Option<String>,
    },
}

/// How a collection lays out when the caller overrides the canonical
/// heuristic (lists inline when at most 6 atoms; attrsets block, `{ }`
/// when empty).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollectionLayout {
    /// One line: `[ a b ]`, `{ a = 1; b = 2; }`. Empty: `[ ]`, `{ }`.
    Inline,
    /// One item per line, even when short. Empty: `[` newline `]`.
    Block,
}

/// Where a `let` body goes relative to `in`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LetLayout {
    /// `in` on its own line, the body on the next (the canonical `Let`).
    BodyOnNextLine,
    /// `in body`: the body hangs off `in` on the same line.
    BodyAfterIn,
}

/// Where a lambda body goes relative to its parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LambdaLayout {
    /// `x: body` (the canonical `Lambda`).
    BodyOnSameLine,
    /// `{ pkgs }:` then the body on the next line at the same indent:
    /// the file-header shape.
    BodyOnNextLine,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AttrPath(pub Vec<AttrKey>);

#[derive(Clone, Debug, PartialEq)]
pub enum AttrKey {
    Ident(String),
    Str(String),
    Interp(NixValue),
}

#[derive(Clone, Debug, PartialEq)]
pub enum LambdaParams {
    Single(String),
    Destructured {
        fields: Vec<ParamField>,
        ellipsis: bool,
        binding: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamField {
    pub name: String,
    pub default: Option<NixValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LetBinding {
    Bind {
        name: String,
        value: NixValue,
    },
    Inherit {
        from: Option<NixValue>,
        names: Vec<String>,
    },
    /// A `# <text>` line between bindings.
    Comment(String),
    /// An empty line between bindings (no indentation).
    Blank,
}

#[derive(Clone, Debug, PartialEq)]
pub enum StrPart {
    Literal(String),
    Interp(NixValue),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NixBinOp {
    Update,
    Concat,
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    Neq,
    Lt,
    Gt,
    Le,
    Ge,
    And,
    Or,
    Impl,
}

impl NixBinOp {
    /// Operator-precedence rank; lower binds tighter. Matches the Nix
    /// language reference; used by the renderer for minimal-parens
    /// output.
    pub const fn precedence(self) -> u8 {
        match self {
            Self::Mul | Self::Div => 6,
            Self::Add | Self::Sub => 7,
            Self::Update => 8,
            Self::Lt | Self::Gt | Self::Le | Self::Ge => 9,
            Self::Eq | Self::Neq => 10,
            Self::And => 11,
            Self::Or => 12,
            Self::Impl => 13,
            Self::Concat => 8,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Update => "//",
            Self::Concat => "++",
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Eq => "==",
            Self::Neq => "!=",
            Self::Lt => "<",
            Self::Gt => ">",
            Self::Le => "<=",
            Self::Ge => ">=",
            Self::And => "&&",
            Self::Or => "||",
            Self::Impl => "->",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NixUnaryOp {
    Neg,
    Not,
}

impl NixUnaryOp {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Neg => "-",
            Self::Not => "!",
        }
    }
}

// ── Builder helpers ──────────────────────────────────────────────────

impl NixValue {
    pub fn str(s: impl Into<String>) -> Self {
        Self::Str(s.into())
    }
    pub fn ident(s: impl Into<String>) -> Self {
        Self::Ident(s.into())
    }
    pub fn attr_path(parts: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::AttrPath(parts.into_iter().map(Into::into).collect())
    }
    pub fn list(items: impl IntoIterator<Item = NixValue>) -> Self {
        Self::List(items.into_iter().collect())
    }
    pub fn attrset(entries: impl IntoIterator<Item = (String, NixValue)>) -> Self {
        Self::AttrSet {
            recursive: false,
            entries: entries
                .into_iter()
                .map(|(k, v)| AttrSetEntry::KeyValue {
                    key: AttrPath(vec![AttrKey::Ident(k)]),
                    value: v,
                })
                .collect(),
        }
    }
    pub fn rec_attrset(entries: impl IntoIterator<Item = (String, NixValue)>) -> Self {
        let mut v = Self::attrset(entries);
        if let Self::AttrSet { recursive, .. } = &mut v {
            *recursive = true;
        }
        v
    }
    pub fn apply(func: NixValue, args: impl IntoIterator<Item = NixValue>) -> Self {
        Self::Apply {
            func: Box::new(func),
            args: args.into_iter().collect(),
        }
    }
    pub fn lambda_single(param: impl Into<String>, body: NixValue) -> Self {
        Self::Lambda {
            params: LambdaParams::Single(param.into()),
            body: Box::new(body),
        }
    }

    /// `base.a.b`, `base.${x}.c`: see [`Self::Select`].
    pub fn select(base: NixValue, path: impl IntoIterator<Item = AttrKey>) -> Self {
        Self::Select {
            base: Box::new(base),
            path: AttrPath(path.into_iter().collect()),
            default: None,
        }
    }
    /// `base.path or default`: see [`Self::Select`].
    pub fn select_or(
        base: NixValue,
        path: impl IntoIterator<Item = AttrKey>,
        default: NixValue,
    ) -> Self {
        Self::Select {
            base: Box::new(base),
            path: AttrPath(path.into_iter().collect()),
            default: Some(Box::new(default)),
        }
    }
    /// Leading comment lines on `value`: see [`Self::Commented`].
    pub fn commented(
        comments: impl IntoIterator<Item = impl Into<String>>,
        value: NixValue,
    ) -> Self {
        Self::Commented {
            comments: comments.into_iter().map(Into::into).collect(),
            value: Box::new(value),
        }
    }
    /// Attrset with an explicit layout.
    pub fn attrset_laid_out(
        layout: CollectionLayout,
        entries: impl IntoIterator<Item = AttrSetEntry>,
    ) -> Self {
        Self::LaidOutAttrSet {
            recursive: false,
            entries: entries.into_iter().collect(),
            layout,
        }
    }
    /// List with an explicit layout.
    pub fn list_laid_out(
        layout: CollectionLayout,
        items: impl IntoIterator<Item = NixValue>,
    ) -> Self {
        Self::LaidOutList {
            items: items.into_iter().collect(),
            layout,
        }
    }
    /// `let ... in` with an explicit layout.
    pub fn let_laid_out(
        layout: LetLayout,
        bindings: impl IntoIterator<Item = LetBinding>,
        body: NixValue,
    ) -> Self {
        Self::LaidOutLet {
            bindings: bindings.into_iter().collect(),
            body: Box::new(body),
            layout,
        }
    }
    /// Lambda with an explicit layout.
    #[must_use]
    pub fn lambda_laid_out(layout: LambdaLayout, params: LambdaParams, body: NixValue) -> Self {
        Self::LaidOutLambda {
            params,
            body: Box::new(body),
            layout,
        }
    }

    /// Render to canonical Nix source. Convenience wrapper around
    /// [`crate::render::render`].
    pub fn render_to_string(&self) -> String {
        crate::render::render(self)
    }
}

/// Convenience: identifier-attrset entry with a string-keyed name.
pub fn entry(key: impl Into<String>, value: NixValue) -> AttrSetEntry {
    AttrSetEntry::KeyValue {
        key: AttrPath(vec![AttrKey::Ident(key.into())]),
        value,
    }
}

/// Convenience: dotted attrset entry — `a.b.c = value;`.
pub fn dotted_entry(dotted: &str, value: NixValue) -> AttrSetEntry {
    AttrSetEntry::KeyValue {
        key: AttrPath(
            dotted
                .split('.')
                .map(|s| AttrKey::Ident(s.to_string()))
                .collect(),
        ),
        value,
    }
}

/// Convenience: `name = value;` let binding.
pub fn bind(name: impl Into<String>, value: NixValue) -> LetBinding {
    LetBinding::Bind {
        name: name.into(),
        value,
    }
}

/// Convenience: `inherit (from) names;` attrset entry; `from = None`
/// gives a bare `inherit names;`.
pub fn inherit(
    from: Option<NixValue>,
    names: impl IntoIterator<Item = impl Into<String>>,
) -> AttrSetEntry {
    AttrSetEntry::Inherit {
        from,
        names: names.into_iter().map(Into::into).collect(),
    }
}

/// Convenience: quoted-key entry, `"aarch64-darwin" = value;`.
pub fn str_entry(key: impl Into<String>, value: NixValue) -> AttrSetEntry {
    AttrSetEntry::KeyValue {
        key: AttrPath(vec![AttrKey::Str(key.into())]),
        value,
    }
}

/// Convenience: destructured lambda parameters without defaults,
/// `{ a, b }` (`ellipsis` appends `...`).
pub fn destructured(
    names: impl IntoIterator<Item = impl Into<String>>,
    ellipsis: bool,
) -> LambdaParams {
    LambdaParams::Destructured {
        fields: names
            .into_iter()
            .map(|n| ParamField {
                name: n.into(),
                default: None,
            })
            .collect(),
        ellipsis,
        binding: None,
    }
}
