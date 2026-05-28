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
