use crate::node::{self, Node};

/// A wrapper around the **Node** trait that allows for serializing and deserializing node trait
/// objects.
#[typetag::serde(tag = "type")]
pub trait SerdeNode {
    fn node(&self) -> &dyn Node;
}

#[typetag::serde]
impl SerdeNode for node::Expr {
    fn node(&self) -> &dyn Node { self }
}

#[typetag::serde]
impl SerdeNode for node::Push<node::Expr> {
    fn node(&self) -> &dyn Node { self }
}

pub mod fn_decl {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(t: &syn::FnDecl, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let item_fn = syn::ItemFn {
            attrs: vec![],
            vis: syn::Visibility::Public(syn::VisPublic { pub_token: Default::default() }),
            constness: None,
            unsafety: None,
            asyncness: None,
            abi: None,
            ident: syn::Ident::new("foo", proc_macro2::Span::call_site()),
            decl: Box::new(t.clone()),
            block: Box::new(syn::Block { stmts: vec![], brace_token: <_>::default() }),
        };
        super::tts::serialize(&item_fn, s)
    }

    pub fn deserialize<'de, D>(d: D) -> Result<syn::FnDecl, D::Error>
    where
        D: Deserializer<'de>,
    {
        let tts = super::tts::deserialize(d)?;
        let syn::ItemFn { decl, .. } = syn::parse_quote!{ #tts };
        Ok(*decl)
    }
}

pub mod fn_attrs {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(t: &Vec<syn::Attribute>, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let syn::ItemFn { decl, .. } = syn::parse_quote!{ fn foo() {} };
        let item_fn = syn::ItemFn {
            attrs: t.clone(),
            vis: syn::Visibility::Public(syn::VisPublic { pub_token: Default::default() }),
            constness: None,
            unsafety: None,
            asyncness: None,
            abi: None,
            ident: syn::Ident::new("foo", proc_macro2::Span::call_site()),
            decl: decl,
            block: Box::new(syn::Block { stmts: vec![], brace_token: <_>::default() }),
        };
        super::tts::serialize(&item_fn, s)
    }

    pub fn deserialize<'de, D>(d: D) -> Result<Vec<syn::Attribute>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let tts = super::tts::deserialize(d)?;
        let syn::ItemFn { attrs, .. } = syn::parse_quote!{ #tts };
        Ok(attrs)
    }
}

pub mod tts {
    use proc_macro2::TokenStream;
    use quote::ToTokens;
    use serde::{Deserialize, Deserializer, Serializer};
    use std::str::FromStr;

    pub fn serialize<T, S>(t: &T, s: S) -> Result<S::Ok, S::Error>
    where
        T: ToTokens,
        S: Serializer,
    {
        let string: String = format!("{}", t.into_token_stream());
        s.serialize_str(&string)
    }

    pub fn deserialize<'de, D>(d: D) -> Result<TokenStream, D::Error>
    where
        D: Deserializer<'de>,
    {
        let string = String::deserialize(d)?;
        Ok(TokenStream::from_str(&string).expect("failed to parse string as token stream"))
    }
}

pub mod ty {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(ty: &syn::Type, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        super::tts::serialize(ty, s)
    }

    pub fn deserialize<'de, D>(d: D) -> Result<syn::Type, D::Error>
    where
        D: Deserializer<'de>,
    {
        let tts = super::tts::deserialize(d)?;
        let ty: syn::Type = syn::parse_quote!{ #tts };
        Ok(ty)
    }
}
