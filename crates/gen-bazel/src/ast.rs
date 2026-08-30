//! Minimal typed Starlark AST. Covers the subset BUILD files actually
//! use: atoms, lists, dicts, function calls, named keyword args,
//! load() statements, assignments. Full Starlark eval is out of
//! scope — this is an emission AST only.

/// One BUILD file statement.
#[derive(Clone, Debug, PartialEq)]
pub enum StarlarkStmt {
    /// `load("@rules/foo.bzl", "sym1", "sym2", ...)`
    Load {
        module: String,
        symbols: Vec<String>,
    },
    /// `func(arg1, arg2, kw=val, ...)`
    Call { func: String, args: Vec<KwArg> },
    /// `name = value`
    Assign { name: String, value: StarlarkValue },
}

/// Argument to a Starlark call. Either positional or keyword. The
/// positional_named variant attaches a name for renderer-side
/// pretty-printing (`name = ...` form), used for the canonical
/// kwarg-style every BUILD file actually authors.
#[derive(Clone, Debug, PartialEq)]
pub enum KwArg {
    Positional(StarlarkValue),
    Named { name: String, value: StarlarkValue },
}

impl KwArg {
    pub fn str(name: &str, value: impl Into<String>) -> Self {
        Self::Named {
            name: name.to_string(),
            value: StarlarkValue::Str(value.into()),
        }
    }
    pub fn positional(value: StarlarkValue) -> Self {
        Self::Positional(value)
    }
    pub fn positional_named(name: &str, value: StarlarkValue) -> Self {
        Self::Named {
            name: name.to_string(),
            value,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StarlarkValue {
    None,
    Bool(bool),
    Int(i64),
    Str(String),
    Ident(String),
    List(Vec<StarlarkValue>),
    Dict(Vec<(String, StarlarkValue)>),
    Call { func: String, args: Vec<KwArg> },
}

impl StarlarkValue {
    pub fn str(s: impl Into<String>) -> Self {
        Self::Str(s.into())
    }
}
