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
