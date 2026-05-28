//! `gen-macros` — proc-macros that auto-implement the universal
//! `gen_types::ecosystem` traits.
//!
//! Materializes the Pillar 12 directive (generation over composition):
//! a new package-manager adapter declares its typed shapes and the
//! macros emit the `Spec` / `QuirkRegistry` impls. The author writes
//! N lines of typed data; the trait surface is mechanical.
//!
//! See `theory/ECOSYSTEM-INTAKE.md` § "The macros" for the contract.
//!
//! ```ignore
//! #[derive(SpecShape, serde::Serialize, serde::Deserialize)]
//! #[spec(args = "BuildRustCrateArgs", quirk = "CrateQuirk")]
//! pub struct BuildSpec {
//!     pub version: u32,
//!     pub crates: indexmap::IndexMap<String, CrateSpec>,
//!     pub root_crate: String,
//!     pub workspace_members: Vec<String>,
//! }
//! ```
//!
//! emits:
//!
//! ```ignore
//! impl gen_types::Spec for BuildSpec {
//!     type Args = BuildRustCrateArgs;
//!     type Quirk = CrateQuirk;
//!     fn schema_version(&self) -> u32 { self.version }
//!     fn root_key(&self) -> &str { self.root_crate.as_str() }
//!     fn member_keys(&self) -> Vec<&str> { self.workspace_members.iter().map(String::as_str).collect() }
//!     fn args_for(&self, key: &str) -> Option<&Self::Args> {
//!         self.crates.get(key).map(|c| &c.build_rust_crate_args)
//!     }
//!     fn quirks_for(&self, key: &str) -> &[Self::Quirk] {
//!         self.crates.get(key).map(|c| c.quirks.as_slice()).unwrap_or(&[])
//!     }
//! }
//! ```

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Fields, Lit, Meta};

/// `#[derive(SpecShape)]` — auto-implement `gen_types::Spec` on a
/// struct whose fields follow the conventional gen build-spec shape:
///
/// - `version: u32`
/// - `root_crate: String` (or `root_key: String`, opt-in via attr)
/// - `workspace_members: Vec<String>`
/// - `crates: IndexMap<String, T>` where `T` carries
///   `build_rust_crate_args: Args` + `quirks: Vec<Quirk>`
///
/// Required attribute:
/// `#[spec(args = "<ArgsTypeName>", quirk = "<QuirkTypeName>")]`
///
/// Optional attributes:
/// `#[spec(args_field = "build_args")]` (default: `build_rust_crate_args`)
/// `#[spec(root_field = "root_key")]`   (default: `root_crate`)
/// `#[spec(members_field = "members")]` (default: `workspace_members`)
/// `#[spec(crates_field = "packages")]` (default: `crates`)
#[proc_macro_derive(SpecShape, attributes(spec))]
pub fn derive_spec_shape(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let mut args_type: Option<String> = None;
    let mut quirk_type: Option<String> = None;
    let mut args_field = "build_rust_crate_args".to_string();
    let mut root_field = "root_crate".to_string();
    let mut members_field = "workspace_members".to_string();
    let mut crates_field = "crates".to_string();

    for attr in &input.attrs {
        if !attr.path().is_ident("spec") {
            continue;
        }
        let Meta::List(list) = &attr.meta else { continue };
        let _ = list.parse_nested_meta(|meta| {
            let Some(ident) = meta.path.get_ident() else {
                return Ok(());
            };
            let value: Lit = meta.value()?.parse()?;
            let Lit::Str(s) = value else {
                return Ok(());
            };
            let v = s.value();
            match ident.to_string().as_str() {
                "args" => args_type = Some(v),
                "quirk" => quirk_type = Some(v),
                "args_field" => args_field = v,
                "root_field" => root_field = v,
                "members_field" => members_field = v,
                "crates_field" => crates_field = v,
                _ => {}
            }
            Ok(())
        });
    }

    let args_type = match args_type {
        Some(t) => syn::parse_str::<syn::Type>(&t).expect("invalid `args` type"),
        None => {
            return TokenStream::from(quote! {
                compile_error!("SpecShape requires `#[spec(args = \"<TypeName>\", quirk = \"<TypeName>\")]`");
            });
        }
    };
    let quirk_type = match quirk_type {
        Some(t) => syn::parse_str::<syn::Type>(&t).expect("invalid `quirk` type"),
        None => {
            return TokenStream::from(quote! {
                compile_error!("SpecShape requires `#[spec(args = \"<TypeName>\", quirk = \"<TypeName>\")]`");
            });
        }
    };

    let args_field_ident = syn::Ident::new(&args_field, proc_macro2::Span::call_site());
    let root_field_ident = syn::Ident::new(&root_field, proc_macro2::Span::call_site());
    let members_field_ident = syn::Ident::new(&members_field, proc_macro2::Span::call_site());
    let crates_field_ident = syn::Ident::new(&crates_field, proc_macro2::Span::call_site());

    let expanded = quote! {
        impl ::gen_types::Spec for #name {
            type Args = #args_type;
            type Quirk = #quirk_type;

            fn schema_version(&self) -> u32 {
                self.version
            }

            fn root_key(&self) -> &str {
                self.#root_field_ident.as_str()
            }

            fn member_keys(&self) -> ::std::vec::Vec<&str> {
                self.#members_field_ident.iter().map(::std::string::String::as_str).collect()
            }

            fn args_for(&self, key: &str) -> ::std::option::Option<&Self::Args> {
                self.#crates_field_ident.get(key).map(|c| &c.#args_field_ident)
            }

            fn quirks_for(&self, key: &str) -> &[Self::Quirk] {
                self.#crates_field_ident
                    .get(key)
                    .map(|c| c.quirks.as_slice())
                    .unwrap_or(&[])
            }
        }
    };

    TokenStream::from(expanded)
}

/// `#[derive(QuirkRegistry)]` — auto-implement
/// `gen_types::QuirkRegistry` on a marker struct that points at the
/// real registry function.
///
/// Required attribute:
/// `#[quirks(enum_name = "<QuirkEnumName>", registry_fn = "module::path::to::registry")]`
///
/// The `registry_fn` must be a `pub fn() -> Vec<(&'static str, Vec<Quirk>)>`
/// the macro can call.
#[proc_macro_derive(QuirkRegistry, attributes(quirks))]
pub fn derive_quirk_registry(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let mut enum_name: Option<String> = None;
    let mut registry_fn: Option<String> = None;
    for attr in &input.attrs {
        if !attr.path().is_ident("quirks") {
            continue;
        }
        let Meta::List(list) = &attr.meta else { continue };
        let _ = list.parse_nested_meta(|meta| {
            let Some(ident) = meta.path.get_ident() else {
                return Ok(());
            };
            let value: Lit = meta.value()?.parse()?;
            let Lit::Str(s) = value else {
                return Ok(());
            };
            let v = s.value();
            match ident.to_string().as_str() {
                "enum_name" => enum_name = Some(v),
                "registry_fn" => registry_fn = Some(v),
                _ => {}
            }
            Ok(())
        });
    }
    let enum_ty = match enum_name {
        Some(t) => syn::parse_str::<syn::Type>(&t).expect("invalid `enum_name`"),
        None => {
            return TokenStream::from(quote! {
                compile_error!("QuirkRegistry requires `#[quirks(enum_name = \"<EnumName>\", registry_fn = \"<path>\")]`");
            });
        }
    };
    let reg_path = match registry_fn {
        Some(t) => syn::parse_str::<syn::Path>(&t).expect("invalid `registry_fn`"),
        None => {
            return TokenStream::from(quote! {
                compile_error!("QuirkRegistry requires `#[quirks(enum_name = \"<EnumName>\", registry_fn = \"<path>\")]`");
            });
        }
    };

    let expanded = quote! {
        impl ::gen_types::QuirkRegistry for #name {
            type Quirk = #enum_ty;

            fn registry() -> ::std::vec::Vec<(&'static str, ::std::vec::Vec<Self::Quirk>)> {
                #reg_path()
            }
        }
    };

    TokenStream::from(expanded)
}

/// `#[derive(TypedDispatcher)]` — auto-implement
/// `gen_types::TypedDispatcher` on a Rust enum whose serde tag is
/// `#[serde(tag = "kind", rename_all = "kebab-case")]`.
///
/// The macro observes the enum's variants and emits a trait impl
/// reflecting the variant universe (kebab-case tags + per-variant
/// field names). Substrate emitters consume the reflection to
/// generate:
///
/// - the Nix `helpers = { ... }` table skeleton for the matching
///   `substrate/lib/build/<eco>/quirk-apply.nix`;
/// - the Lisp catalog entry naming the dispatcher;
/// - a coverage test asserting every variant has a consumer arm.
///
/// Only unit variants and named-field struct variants are supported
/// (the serde-tagged-enum shape pleme-io uses universally). Tuple
/// variants raise a compile error.
#[proc_macro_derive(TypedDispatcher)]
pub fn derive_typed_dispatcher(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    let Data::Enum(data) = &input.data else {
        return TokenStream::from(quote! {
            compile_error!("#[derive(TypedDispatcher)] only works on enums");
        });
    };

    let mut kind_entries: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut field_entries: Vec<proc_macro2::TokenStream> = Vec::new();

    for variant in &data.variants {
        let tag = to_kebab_case(&variant.ident.to_string());
        let fields = match &variant.fields {
            Fields::Named(named) => named
                .named
                .iter()
                .filter_map(|f| f.ident.as_ref().map(std::string::ToString::to_string))
                .collect::<Vec<_>>(),
            Fields::Unit => Vec::new(),
            Fields::Unnamed(_) => {
                let msg = format!(
                    "#[derive(TypedDispatcher)] variant `{}` uses tuple fields; only named-field and unit variants are supported (matches the serde-tagged-enum shape pleme-io requires)",
                    variant.ident
                );
                return TokenStream::from(quote! {
                    compile_error!(#msg);
                });
            }
        };

        kind_entries.push(quote! { #tag });
        let field_strs: Vec<proc_macro2::TokenStream> =
            fields.iter().map(|f| quote! { #f }).collect();
        field_entries.push(quote! {
            (#tag, ::std::vec![ #( #field_strs ),* ])
        });
    }

    let expanded = quote! {
        impl ::gen_types::TypedDispatcher for #name {
            fn variant_kinds() -> ::std::vec::Vec<&'static str> {
                ::std::vec![ #( #kind_entries ),* ]
            }

            fn variant_fields() -> ::std::vec::Vec<(&'static str, ::std::vec::Vec<&'static str>)> {
                ::std::vec![ #( #field_entries ),* ]
            }
        }
    };

    TokenStream::from(expanded)
}

// ── Discriminant + IsVariant — typed-reflection derive surface ───
//
// These two derives are the substrate-wide derive surface for typed
// enums (PATTERN-EXTRACTION.md Patterns 6 + sibling). They live in
// gen-macros (next to TypedDispatcher) so consumers in any pleme-io
// crate that already depends on gen-platform can reach for them
// without adding a fresh derive crate.
//
// Discriminant emits `pub const fn <method>(&self) -> &'static str`
// returning the variant name as a stable lowercase / kebab-case /
// snake_case / title-case identifier.
//
// IsVariant emits `pub const fn is_<variant>(&self) -> bool`
// per variant.
//
// Both support per-variant `#[discriminant(name = "...")]` /
// `#[is_variant(name = "...")]` overrides for cases where the
// auto-derived name doesn't match the historical wire format.

#[derive(Clone, Copy)]
enum DiscriminantCase {
    Kebab,
    Snake,
    Lower,
    Title,
}

impl DiscriminantCase {
    fn apply(self, s: &str) -> String {
        match self {
            DiscriminantCase::Kebab => to_kebab_case(s),
            DiscriminantCase::Snake => discriminant_to_snake(s),
            DiscriminantCase::Lower => s.to_ascii_lowercase(),
            DiscriminantCase::Title => s.to_string(),
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "kebab" | "kebab-case" => Some(DiscriminantCase::Kebab),
            "snake" | "snake_case" => Some(DiscriminantCase::Snake),
            "lower" | "lowercase" => Some(DiscriminantCase::Lower),
            "title" | "Title" | "TitleCase" => Some(DiscriminantCase::Title),
            _ => None,
        }
    }
}

fn discriminant_to_snake(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn discriminant_variant_pattern(v: &syn::Variant) -> proc_macro2::TokenStream {
    let name = &v.ident;
    match &v.fields {
        Fields::Unit => quote! { Self::#name },
        Fields::Unnamed(_) => quote! { Self::#name(..) },
        Fields::Named(_) => quote! { Self::#name { .. } },
    }
}

fn discriminant_variant_explicit_name(v: &syn::Variant) -> Option<String> {
    for attr in &v.attrs {
        if !attr.path().is_ident("discriminant") {
            continue;
        }
        let mut out = None;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                let value = meta.value()?;
                let s: syn::LitStr = value.parse()?;
                out = Some(s.value());
            }
            Ok(())
        });
        if out.is_some() {
            return out;
        }
    }
    None
}

/// `#[derive(Discriminant)]` — auto-implement
/// `pub const fn <method>(&self) -> &'static str` returning the
/// variant name as a stable case-folded identifier.
///
/// # Attributes
///
/// - `#[discriminant(method = "kind")]` — method name (default
///   `"discriminant"`)
/// - `#[discriminant(case = "kebab" | "snake" | "lower" | "title")]`
///   — variant-name case transformation (default `"kebab"`)
/// - `#[discriminant(also_display)]` — also emit `impl Display`
///   delegating to the method (writes the variant string to the
///   formatter). Eliminates the boilerplate Display impl that
///   recurs across the substrate for typed enums where Display
///   IS the discriminant.
/// - Per-variant `#[discriminant(name = "explicit-name")]` overrides
///   the auto-derived name (used when the wire format pre-dates the
///   rule).
///
/// Compounding: pairs naturally with `#[derive(IsVariant)]` (predicate
/// methods) and `#[derive(TypedDispatcher)]` (variant → consumer arm
/// dispatch). All three target the same closed-variant-universe shape
/// the pleme-io substrate uses everywhere.
#[proc_macro_derive(Discriminant, attributes(discriminant))]
pub fn derive_discriminant(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let enum_name = input.ident.clone();

    let Data::Enum(de) = input.data.clone() else {
        return syn::Error::new_spanned(
            &enum_name,
            "#[derive(Discriminant)] is only valid on enums",
        )
        .to_compile_error()
        .into();
    };

    let mut method = "discriminant".to_string();
    let mut case = DiscriminantCase::Kebab;
    let mut also_display = false;
    for attr in &input.attrs {
        if !attr.path().is_ident("discriminant") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("method") {
                let value = meta.value()?;
                let s: syn::LitStr = value.parse()?;
                method = s.value();
            } else if meta.path.is_ident("case") {
                let value = meta.value()?;
                let s: syn::LitStr = value.parse()?;
                if let Some(c) = DiscriminantCase::parse(&s.value()) {
                    case = c;
                }
            } else if meta.path.is_ident("also_display") {
                also_display = true;
            }
            Ok(())
        });
    }
    let method_ident = syn::Ident::new(&method, proc_macro2::Span::call_site());

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let arms: Vec<proc_macro2::TokenStream> = de
        .variants
        .iter()
        .map(|v| {
            let pattern = discriminant_variant_pattern(v);
            let name_str = discriminant_variant_explicit_name(v)
                .unwrap_or_else(|| case.apply(&v.ident.to_string()));
            quote! { #pattern => #name_str }
        })
        .collect();

    let display_impl = if also_display {
        quote! {
            impl #impl_generics ::core::fmt::Display for #enum_name #ty_generics #where_clause {
                fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                    f.write_str(self.#method_ident())
                }
            }
        }
    } else {
        quote! {}
    };

    let expanded = quote! {
        impl #impl_generics #enum_name #ty_generics #where_clause {
            /// Stable variant discriminant — auto-generated by
            /// `#[derive(Discriminant)]`. The string IS the wire
            /// identifier for metrics labels / audit-log tags /
            /// rate-limit keys; renaming an existing variant is a
            /// breaking change.
            pub const fn #method_ident(&self) -> &'static str {
                match self {
                    #(#arms),*
                }
            }
        }
        #display_impl
    };

    expanded.into()
}

/// `#[derive(FromStrKind)]` — the inverse of Discriminant. Parses
/// a string back to a variant using the same case-folded variant
/// name. Only unit variants are supported (data variants need
/// caller-supplied payloads — out of scope for a string-only parse).
///
/// # Attributes
///
/// - `#[from_str_kind(case = "kebab" | "snake" | "lower" | "title")]`
///   — case transform matching the wire format (default `"kebab"`)
/// - Per-variant `#[from_str_kind(name = "explicit")]` — match a
///   specific wire string for this variant (overrides case transform)
/// - `#[from_str_kind(error = "MyEnumParseError")]` — name of the
///   generated error type (default `<EnumName>ParseError`)
///
/// Pairs with Discriminant: when both derives are on the same enum
/// with the same case transform, `s.parse() -> Ok(v); v.discriminant() == s`
/// — a typed round-trip.
#[proc_macro_derive(FromStrKind, attributes(from_str_kind))]
pub fn derive_from_str_kind(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let enum_name = input.ident.clone();

    let Data::Enum(de) = input.data.clone() else {
        return syn::Error::new_spanned(
            &enum_name,
            "#[derive(FromStrKind)] is only valid on enums",
        )
        .to_compile_error()
        .into();
    };

    let mut case = DiscriminantCase::Kebab;
    let mut error_name = format!("{enum_name}ParseError");
    for attr in &input.attrs {
        if !attr.path().is_ident("from_str_kind") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("case") {
                let value = meta.value()?;
                let s: syn::LitStr = value.parse()?;
                if let Some(c) = DiscriminantCase::parse(&s.value()) {
                    case = c;
                }
            } else if meta.path.is_ident("error") {
                let value = meta.value()?;
                let s: syn::LitStr = value.parse()?;
                error_name = s.value();
            }
            Ok(())
        });
    }
    let error_ident = syn::Ident::new(&error_name, proc_macro2::Span::call_site());

    let mut arms: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut known_strings: Vec<String> = Vec::new();
    for v in &de.variants {
        if !matches!(v.fields, Fields::Unit) {
            return syn::Error::new_spanned(
                &v.ident,
                "#[derive(FromStrKind)] requires all variants to be unit variants (no data payloads)",
            )
            .to_compile_error()
            .into();
        }
        let v_ident = &v.ident;
        let explicit = v.attrs.iter().find_map(|attr| {
            if !attr.path().is_ident("from_str_kind") {
                return None;
            }
            let mut out = None;
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("name") {
                    let value = meta.value()?;
                    let s: syn::LitStr = value.parse()?;
                    out = Some(s.value());
                }
                Ok(())
            });
            out
        });
        let name_str = explicit.unwrap_or_else(|| case.apply(&v_ident.to_string()));
        known_strings.push(name_str.clone());
        arms.push(quote! { #name_str => Ok(Self::#v_ident) });
    }

    let known_list = known_strings.join(" | ");
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let expanded = quote! {
        /// Auto-generated parse error for the matching `FromStrKind` impl.
        #[derive(Debug, Clone, PartialEq, Eq)]
        pub struct #error_ident {
            pub input: ::std::string::String,
        }

        impl ::core::fmt::Display for #error_ident {
            fn fmt(&self, f: &mut ::core::fmt::Formatter<'_>) -> ::core::fmt::Result {
                write!(
                    f,
                    "unknown variant {input:?}; expected one of: {known}",
                    input = self.input,
                    known = #known_list,
                )
            }
        }

        impl ::std::error::Error for #error_ident {}

        impl #impl_generics ::core::str::FromStr for #enum_name #ty_generics #where_clause {
            type Err = #error_ident;
            fn from_str(s: &str) -> ::core::result::Result<Self, Self::Err> {
                match s {
                    #(#arms),*,
                    other => Err(#error_ident { input: other.to_string() }),
                }
            }
        }
    };

    expanded.into()
}

fn is_variant_method_name(v: &syn::Variant) -> syn::Ident {
    let explicit = v.attrs.iter().find_map(|attr| {
        if !attr.path().is_ident("is_variant") {
            return None;
        }
        let mut out = None;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                let value = meta.value()?;
                let s: syn::LitStr = value.parse()?;
                out = Some(s.value());
            }
            Ok(())
        });
        out
    });
    let snake = explicit.unwrap_or_else(|| discriminant_to_snake(&v.ident.to_string()));
    syn::Ident::new(&format!("is_{snake}"), proc_macro2::Span::call_site())
}

/// `#[derive(IsVariant)]` — auto-implement `pub const fn is_<variant>(&self) -> bool`
/// for every variant.
///
/// # Attributes
///
/// - Per-variant `#[is_variant(name = "explicit")]` overrides the
///   auto-derived method-name suffix (default is the snake-cased
///   variant identifier).
///
/// Compounding: pairs with `#[derive(Discriminant)]` for variant→name
/// reflection and `#[derive(TypedDispatcher)]` for variant→consumer
/// dispatch.
#[proc_macro_derive(IsVariant, attributes(is_variant))]
pub fn derive_is_variant(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let enum_name = input.ident.clone();

    let Data::Enum(de) = input.data.clone() else {
        return syn::Error::new_spanned(
            &enum_name,
            "#[derive(IsVariant)] is only valid on enums",
        )
        .to_compile_error()
        .into();
    };

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let methods: Vec<proc_macro2::TokenStream> = de
        .variants
        .iter()
        .map(|v| {
            let pattern = discriminant_variant_pattern(v);
            let method_name = is_variant_method_name(v);
            quote! {
                pub const fn #method_name(&self) -> bool {
                    matches!(self, #pattern)
                }
            }
        })
        .collect();

    let expanded = quote! {
        impl #impl_generics #enum_name #ty_generics #where_clause {
            #(#methods)*
        }
    };

    expanded.into()
}

/// `#[derive(BackendError)]` — auto-implement a trait with the
/// shape:
///
/// ```ignore
/// pub trait BackendError {
///     fn is_retryable(&self) -> bool;
///     fn is_auth_failure(&self) -> bool { false }
///     fn kind(&self) -> &'static str;
/// }
/// ```
///
/// The derive emits:
///   - `is_retryable`  — `true` for variants tagged `#[backend_error(transient)]`
///   - `is_auth_failure` — `true` for variants tagged `#[backend_error(auth)]`
///   - `kind` — delegates to `self.discriminant()` (requires
///     `#[derive(Discriminant)]` to be on the same enum with method
///     `discriminant` OR the consumer overrides via `kind_method`)
///
/// # Attributes
///
/// - `#[backend_error(trait_path = "::path::to::BackendError")]` —
///   fully-qualified trait path. Defaults to unqualified
///   `BackendError` — consumer must `use the_trait::BackendError` in
///   scope.
/// - `#[backend_error(kind_method = "kind")]` — name of the
///   `&'static str`-returning method to delegate `kind()` to (default
///   `"discriminant"` — matches the default Discriminant method name).
/// - Per-variant `#[backend_error(transient)]` — variant is transient
///   (caller should retry).
/// - Per-variant `#[backend_error(auth)]` — variant is an auth
///   failure (HTTP 401/403 maps).
/// - Per-variant `#[backend_error(permanent)]` — variant is permanent
///   (caller must not retry). Default for unattributed variants.
///
/// # Round-trip with Discriminant
///
/// Pairs with Discriminant to deliver the BackendError contract in
/// two derives:
///
/// ```ignore
/// use magma_converge::BackendError;  // trait in scope
///
/// #[derive(Debug, thiserror::Error, gen_platform::Discriminant, gen_platform::BackendError)]
/// #[discriminant(method = "discriminant", case = "snake")]
/// enum BlobStoreError {
///     #[error("not found at {path:?}")]
///     NotFound { path: String },
///
///     #[error("permission denied at {path:?}")]
///     #[backend_error(auth)]
///     PermissionDenied { path: String },
///
///     #[error("transient at {path:?}")]
///     #[backend_error(transient)]
///     Transient { path: String },
///
///     #[error("permanent at {path:?}")]
///     Permanent { path: String },
/// }
///
/// // Auto-generated:
/// //   impl BackendError for BlobStoreError {
/// //       fn is_retryable(&self) -> bool { matches!(self, Self::Transient { .. }) }
/// //       fn is_auth_failure(&self) -> bool { matches!(self, Self::PermissionDenied { .. }) }
/// //       fn kind(&self) -> &'static str { self.discriminant() }
/// //   }
/// ```
#[proc_macro_derive(BackendError, attributes(backend_error))]
pub fn derive_backend_error(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let enum_name = input.ident.clone();

    let Data::Enum(de) = input.data.clone() else {
        return syn::Error::new_spanned(
            &enum_name,
            "#[derive(BackendError)] is only valid on enums",
        )
        .to_compile_error()
        .into();
    };

    let mut trait_path: syn::Path = syn::parse_quote!(BackendError);
    let mut kind_method = "discriminant".to_string();
    for attr in &input.attrs {
        if !attr.path().is_ident("backend_error") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("trait_path") {
                let value = meta.value()?;
                let s: syn::LitStr = value.parse()?;
                if let Ok(p) = syn::parse_str::<syn::Path>(&s.value()) {
                    trait_path = p;
                }
            } else if meta.path.is_ident("kind_method") {
                let value = meta.value()?;
                let s: syn::LitStr = value.parse()?;
                kind_method = s.value();
            }
            Ok(())
        });
    }
    let kind_method_ident = syn::Ident::new(&kind_method, proc_macro2::Span::call_site());

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let mut transient_patterns: Vec<proc_macro2::TokenStream> = Vec::new();
    let mut auth_patterns: Vec<proc_macro2::TokenStream> = Vec::new();
    for v in &de.variants {
        let mut tags: std::collections::HashSet<String> = std::collections::HashSet::new();
        for attr in &v.attrs {
            if !attr.path().is_ident("backend_error") {
                continue;
            }
            let _ = attr.parse_nested_meta(|meta| {
                if let Some(ident) = meta.path.get_ident() {
                    tags.insert(ident.to_string());
                }
                Ok(())
            });
        }
        let pattern = discriminant_variant_pattern(v);
        if tags.contains("transient") {
            transient_patterns.push(pattern.clone());
        }
        if tags.contains("auth") {
            auth_patterns.push(pattern.clone());
        }
    }

    let is_retryable_body = if transient_patterns.is_empty() {
        quote! { false }
    } else {
        quote! { matches!(self, #(#transient_patterns)|*) }
    };
    let is_auth_failure_body = if auth_patterns.is_empty() {
        quote! { false }
    } else {
        quote! { matches!(self, #(#auth_patterns)|*) }
    };

    let expanded = quote! {
        impl #impl_generics #trait_path for #enum_name #ty_generics #where_clause {
            fn is_retryable(&self) -> bool {
                #is_retryable_body
            }

            fn is_auth_failure(&self) -> bool {
                #is_auth_failure_body
            }

            fn kind(&self) -> &'static str {
                self.#kind_method_ident()
            }
        }
    };

    expanded.into()
}

/// Convert PascalCase variant identifiers to kebab-case serde tags.
/// Mirrors `#[serde(rename_all = "kebab-case")]` semantics via
/// heck-style word boundaries: lower→upper and digit→upper both
/// trigger a hyphen. `Wasm32Wasi` → `wasm32-wasi`.
fn to_kebab_case(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    let mut prev_lower = false;
    let mut prev_digit = false;
    for ch in s.chars() {
        if ch.is_ascii_uppercase() {
            if prev_lower || prev_digit {
                out.push('-');
            }
            for c in ch.to_lowercase() {
                out.push(c);
            }
            prev_lower = false;
            prev_digit = false;
        } else if ch.is_ascii_digit() {
            out.push(ch);
            prev_lower = false;
            prev_digit = true;
        } else {
            out.push(ch);
            prev_lower = true;
            prev_digit = false;
        }
    }
    out
}
