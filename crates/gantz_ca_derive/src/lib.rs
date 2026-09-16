//! Derive macro for the `CaHash` trait.
//!
//! ```ignore
//! use gantz_ca::CaHash;
//!
//! #[derive(CaHash)]
//! #[cahash("my.type")]
//! pub struct MyType {
//!     field: String,
//!     #[cahash(skip)]
//!     cached: Option<usize>,
//! }
//! ```
//!
//! See the [`CaHash`](derive.CaHash.html) macro documentation for details.

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    Data, DeriveInput, Fields, GenericParam, Generics, Ident, Index, Lit, Meta, parse_macro_input,
};

/// Derive macro for `CaHash`.
///
/// Implements content-addressable hashing for structs and enums. The
/// generated impl hashes the optional discriminator, then each field that is
/// not skipped, in declaration order.
///
/// # Attributes
///
/// - `#[cahash("discriminator")]` on the type. A unique string prefix for
///   this type's hash. Use dotted lowercase names like `gantz.my-type`. When
///   omitted, only the fields are hashed. This suits wrapper types.
///
/// - `#[cahash(skip)]` on a field. Excludes the field from hashing. This
///   suits `PhantomData`, cached values and UI-only state.
///
/// # Generated Code Examples
///
/// ## Structs
///
/// ```ignore
/// #[derive(CaHash)]
/// #[cahash("gantz.expr")]
/// pub struct Expr {
///     src: String,
/// }
/// ```
///
/// Generates:
///
/// ```ignore
/// impl gantz_ca::CaHash for Expr {
///     fn hash(&self, hasher: &mut gantz_ca::Hasher) {
///         hasher.update("gantz.expr".as_bytes());
///         gantz_ca::CaHash::hash(&self.src, hasher);
///     }
/// }
/// ```
///
/// Tuple struct fields are hashed by index in the same way. A unit struct
/// hashes only its discriminator.
///
/// ## Generic Types and Skipped Fields
///
/// Type parameters used in non-skipped fields receive a `CaHash` bound.
/// Skipped fields do not contribute to the bounds.
///
/// ```ignore
/// #[derive(CaHash)]
/// #[cahash("gantz.state")]
/// pub struct State<Env, N, S> {
///     #[cahash(skip)]
///     pub env: PhantomData<Env>,
///     pub node: N,
///     #[cahash(skip)]
///     pub state: PhantomData<S>,
/// }
/// ```
///
/// Generates:
///
/// ```ignore
/// // Note: Only `N` gets a CaHash bound since Env and S are in skipped fields.
/// impl<N: gantz_ca::CaHash> gantz_ca::CaHash for State<Env, N, S> {
///     fn hash(&self, hasher: &mut gantz_ca::Hasher) {
///         hasher.update("gantz.state".as_bytes());
///         gantz_ca::CaHash::hash(&self.node, hasher);
///     }
/// }
/// ```
///
/// ## Enums
///
/// Variants are tagged with sequential `u8` values starting from 0. Named
/// variant fields are hashed in declaration order in the same way.
///
/// ```ignore
/// #[derive(CaHash)]
/// #[cahash("gantz.eval-conf")]
/// pub enum EvalConf {
///     All,           // tag = 0
///     Set(Conns),    // tag = 1
/// }
/// ```
///
/// Generates:
///
/// ```ignore
/// impl gantz_ca::CaHash for EvalConf {
///     fn hash(&self, hasher: &mut gantz_ca::Hasher) {
///         hasher.update("gantz.eval-conf".as_bytes());
///         match self {
///             Self::All => {
///                 hasher.update(&[0u8]);
///             }
///             Self::Set(f0) => {
///                 hasher.update(&[1u8]);
///                 gantz_ca::CaHash::hash(f0, hasher);
///             }
///         }
///     }
/// }
/// ```
///
/// # Errors
///
/// Compilation fails for a union type.
#[proc_macro_derive(CaHash, attributes(cahash))]
pub fn derive_ca_hash(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    match derive_ca_hash_impl(input) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn derive_ca_hash_impl(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = &input.ident;
    let discriminator = find_discriminator(&input);
    let generics = add_ca_hash_bounds(&input.generics, &input.data);
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let hash_body = match &input.data {
        Data::Struct(data) => generate_struct_hash(&data.fields)?,
        Data::Enum(data) => generate_enum_hash(data)?,
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                input,
                "CaHash cannot be derived for unions",
            ));
        }
    };

    let discriminator_hash = discriminator.map(|d| {
        quote! { hasher.update(#d.as_bytes()); }
    });

    Ok(quote! {
        impl #impl_generics gantz_ca::CaHash for #name #ty_generics #where_clause {
            fn hash(&self, hasher: &mut gantz_ca::Hasher) {
                #discriminator_hash
                #hash_body
            }
        }
    })
}

/// Find the `#[cahash("discriminator")]` attribute, if present.
fn find_discriminator(input: &DeriveInput) -> Option<String> {
    for attr in &input.attrs {
        if attr.path().is_ident("cahash") {
            if let Ok(Lit::Str(s)) = attr.parse_args::<Lit>() {
                return Some(s.value());
            }
        }
    }
    None
}

/// Add `CaHash` bounds to type parameters that are used in non-skipped fields.
fn add_ca_hash_bounds(generics: &Generics, data: &Data) -> Generics {
    let mut generics = generics.clone();

    let type_params: Vec<Ident> = generics
        .params
        .iter()
        .filter_map(|p| {
            if let GenericParam::Type(t) = p {
                Some(t.ident.clone())
            } else {
                None
            }
        })
        .collect();

    let used_params: Vec<&Ident> = match data {
        Data::Struct(data) => collect_used_type_params(&data.fields, &type_params),
        Data::Enum(data) => {
            let mut used = Vec::new();
            for variant in &data.variants {
                used.extend(collect_used_type_params(&variant.fields, &type_params));
            }
            used
        }
        Data::Union(_) => vec![],
    };

    for param in &mut generics.params {
        if let GenericParam::Type(type_param) = param {
            if used_params.iter().any(|p| *p == &type_param.ident) {
                type_param.bounds.push(syn::parse_quote!(gantz_ca::CaHash));
            }
        }
    }

    generics
}

/// Collect type parameter identifiers used in non-skipped fields.
fn collect_used_type_params<'a>(fields: &Fields, type_params: &'a [Ident]) -> Vec<&'a Ident> {
    let mut used = Vec::new();

    for field in fields.iter() {
        if should_skip_field(field) {
            continue;
        }

        // Match each type parameter by name against the field type's tokens.
        let ty_string = quote!(#field.ty).to_string();
        for param in type_params {
            if ty_string.contains(&param.to_string()) {
                used.push(param);
            }
        }
    }

    used
}

/// Check if a field has `#[cahash(skip)]`.
fn should_skip_field(field: &syn::Field) -> bool {
    for attr in &field.attrs {
        if attr.path().is_ident("cahash") {
            if let Ok(Meta::Path(path)) = attr.parse_args::<Meta>() {
                if path.is_ident("skip") {
                    return true;
                }
            }
        }
    }
    false
}

/// Generate hash calls for struct fields.
fn generate_struct_hash(fields: &Fields) -> syn::Result<proc_macro2::TokenStream> {
    match fields {
        Fields::Named(fields) => {
            let hash_calls = fields
                .named
                .iter()
                .filter(|f| !should_skip_field(f))
                .map(|f| {
                    let name = &f.ident;
                    quote! {
                        gantz_ca::CaHash::hash(&self.#name, hasher);
                    }
                });
            Ok(quote! { #(#hash_calls)* })
        }
        Fields::Unnamed(fields) => {
            let hash_calls = fields
                .unnamed
                .iter()
                .enumerate()
                .filter(|(_, f)| !should_skip_field(f))
                .map(|(i, _)| {
                    let index = Index::from(i);
                    quote! {
                        gantz_ca::CaHash::hash(&self.#index, hasher);
                    }
                });
            Ok(quote! { #(#hash_calls)* })
        }
        Fields::Unit => Ok(quote! {}),
    }
}

/// Generate hash calls for enum variants.
fn generate_enum_hash(data: &syn::DataEnum) -> syn::Result<proc_macro2::TokenStream> {
    let arms = data.variants.iter().enumerate().map(|(i, variant)| {
        let variant_name = &variant.ident;
        let tag = i as u8;

        match &variant.fields {
            Fields::Unit => {
                quote! {
                    Self::#variant_name => {
                        hasher.update(&[#tag]);
                    }
                }
            }
            Fields::Unnamed(fields) => {
                let bindings: Vec<_> = (0..fields.unnamed.len())
                    .map(|i| {
                        let name = Ident::new(&format!("f{}", i), proc_macro2::Span::call_site());
                        name
                    })
                    .collect();
                let hash_calls = fields
                    .unnamed
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| !should_skip_field(f))
                    .map(|(i, _)| {
                        let binding = &bindings[i];
                        quote! {
                            gantz_ca::CaHash::hash(#binding, hasher);
                        }
                    });
                quote! {
                    Self::#variant_name(#(#bindings),*) => {
                        hasher.update(&[#tag]);
                        #(#hash_calls)*
                    }
                }
            }
            Fields::Named(fields) => {
                let field_names: Vec<_> = fields
                    .named
                    .iter()
                    .map(|f| f.ident.as_ref().unwrap())
                    .collect();
                let hash_calls = fields
                    .named
                    .iter()
                    .filter(|f| !should_skip_field(f))
                    .map(|f| {
                        let name = &f.ident;
                        quote! {
                            gantz_ca::CaHash::hash(#name, hasher);
                        }
                    });
                quote! {
                    Self::#variant_name { #(#field_names),* } => {
                        hasher.update(&[#tag]);
                        #(#hash_calls)*
                    }
                }
            }
        }
    });

    Ok(quote! {
        match self {
            #(#arms)*
        }
    })
}
