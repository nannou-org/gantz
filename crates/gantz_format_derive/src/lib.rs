//! Derive macro for the `NodeTag` trait.
//!
//! ```ignore
//! use gantz_format::NodeTag;
//!
//! // The wire tag defaults to the type's name: "Bang".
//! #[derive(NodeTag)]
//! pub struct Bang;
//!
//! // Or override it with the `tag` attribute.
//! #[derive(NodeTag)]
//! #[tag("my.custom-tag")]
//! pub struct Custom;
//! ```
//!
//! See the [`NodeTag`](derive.NodeTag.html) macro documentation for details.

use proc_macro::TokenStream;
use quote::quote;
use syn::{DeriveInput, LitStr, parse_macro_input};

/// Derive macro for `NodeTag`.
///
/// Implements `gantz_format::NodeTag` with the type's name as its wire tag
/// (the `"type"` entry of the node's serialized map form).
///
/// # Attributes
///
/// - `#[tag("...")]` (optional on type): Override the tag. Tags are part of
///   the wire format - changing one breaks the loading of existing `.gantz`
///   exports and persisted registries that contain the node.
///
/// # Generated Code Example
///
/// ```ignore
/// #[derive(NodeTag)]
/// #[tag("my.custom-tag")]
/// pub struct Custom;
/// ```
///
/// Generates:
///
/// ```ignore
/// impl ::gantz_format::NodeTag for Custom {
///     const TAG: &'static str = "my.custom-tag";
/// }
/// ```
#[proc_macro_derive(NodeTag, attributes(tag))]
pub fn node_tag(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    derive_node_tag(&input)
        .unwrap_or_else(|e| e.to_compile_error())
        .into()
}

fn derive_node_tag(input: &DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;
    let mut tag = None;
    for attr in &input.attrs {
        if !attr.path().is_ident("tag") {
            continue;
        }
        if tag.is_some() {
            let msg = "duplicate `#[tag(..)]` attribute";
            return Err(syn::Error::new_spanned(attr, msg));
        }
        let lit = attr.parse_args::<LitStr>().map_err(|_| {
            let msg = "expected a string literal tag: `#[tag(\"MyTag\")]`";
            syn::Error::new_spanned(attr, msg)
        })?;
        tag = Some(lit.value());
    }
    let tag = tag.unwrap_or_else(|| name.to_string());
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();
    Ok(quote! {
        impl #impl_generics ::gantz_format::NodeTag for #name #ty_generics #where_clause {
            const TAG: &'static str = #tag;
        }
    })
}
