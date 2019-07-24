//! Implementation of the `GantzNode` custom derive macro.

use proc_macro2::{Span, TokenStream, TokenTree};
use quote::ToTokens;
use syn;

#[derive(Debug)]
struct Inlet {
    // The name of the inlet field.
    name: syn::Ident,
    // The type of the inlet field.
    ty: syn::Type,
    // An optionally specified function for processing a value received on an inlet.
    process_fn: Option<String>,
}

#[derive(Debug)]
struct Outlet {
    // The name of the outlet variant.
    name: syn::Ident,
    // The type of the outlet variant.
    ty: syn::Type,
    // The callable name/path for the function used for producing the outlet value.
    process_fn: String,
}

// Check the given attrs for an attribute with the given `ident`.
// If there is one, return the source code describing the inlets struct.
fn src_from_attr(attrs: &[syn::Attribute], ident: &str) -> Option<String> {
    attrs
        .iter()
        .find(|attr| {
            attr.path
                .segments
                .iter()
                .next()
                .map(|segment| segment.ident == ident)
                .unwrap_or(false)
        })
        .map(|attr| {
            let mut iter = attr.tts.clone().into_iter();

            // Check for the "equals" symbol.
            match iter.next() {
                Some(TokenTree::Punct(ref p)) if p.as_char() == '=' => (),
                t => panic!(
                    "unexpected token when parsing `{}` attr:\nexpected: `=`\nfound: {:?}",
                    ident, t
                ),
            }

            // Retrieve the source string.
            match iter.next() {
                Some(TokenTree::Literal(lit)) => {
                    let src_with_quotes = format!("{}", lit);
                    let src_lit_str: syn::LitStr = syn::parse_str(&src_with_quotes).unwrap();
                    let src_string = src_lit_str.value();
                    src_string
                }
                t => panic!(
                    "unexpected token when parsing `{}` attr:\nexpected source string\nfound: {:?}",
                    ident, t
                ),
            }
        })
}

// Check the given attrs for an `inlets` attribute. If there is one, return the source code
// describing the inlets struct.
fn inlets_src(attrs: &[syn::Attribute]) -> Option<String> {
    src_from_attr(attrs, "inlets")
}

// Check the given attrs for an `inlets` attribute. If there is one, return the source code
// describing the inlets struct.
fn outlets_src(attrs: &[syn::Attribute]) -> Option<String> {
    src_from_attr(attrs, "outlets")
}

// Find the ident of the "process fn" from the attribute with the given name if one exists.
fn attr_process_fn(
    node_attrs: &[syn::Attribute],
    attr_ident: &str,
    io_ident: &syn::Ident,
) -> Option<String> {
    node_attrs
        .iter()
        .filter(|attr| {
            attr.path
                .segments
                .iter()
                .next()
                .map(|segment| segment.ident == attr_ident)
                .unwrap_or(false)
        })
        .filter_map(|attr| {
            // Retrieve the token stream from the group within the parens.
            let tts = match attr.tts.clone().into_iter().next() {
                Some(TokenTree::Group(group)) => group.stream(),
                t => panic!(
                    "unexpected token when parsing `{}` attr: {:?}",
                    attr_ident, t
                ),
            };

            // Check one iter elem at a time.
            let mut iter = tts.into_iter();

            // Check for the matching field name.
            match iter.next() {
                Some(TokenTree::Ident(ref ident)) if ident == io_ident => (),
                _ => return None,
            }

            // Check for the "equals" symbol.
            match iter.next() {
                Some(TokenTree::Punct(ref p)) if p.as_char() == '=' => (),
                t => panic!(
                    "unexpected token when parsing `{}` attr:\nexpected: `=`\nfound: {:?}",
                    attr_ident, t
                ),
            }

            // Retrieve the process function name as a `String`.
            match iter.next() {
                Some(TokenTree::Literal(lit)) => {
                    let process_fn_with_quotes = format!("{}", lit);
                    let process_fn_lit_str: syn::LitStr =
                        syn::parse_str(&process_fn_with_quotes).unwrap();
                    Some(process_fn_lit_str.value())
                }
                t => panic!(
                    "unexpected token when parsing `{}` attr:\nexpected source string\nfound: {:?}",
                    attr_ident, t
                ),
            }
        })
        .next()
}

// Find the ident of the "process fn" for the inlet if one exists.
fn inlet_process_fn(node_attrs: &[syn::Attribute], field_ident: &syn::Ident) -> Option<String> {
    attr_process_fn(node_attrs, "process_inlet", field_ident)
}

// Find the ident of the "process fn" for the outlet.
fn outlet_process_fn(node_attrs: &[syn::Attribute], variant_ident: &syn::Ident) -> String {
    match attr_process_fn(node_attrs, "process_outlet", variant_ident) {
        Some(string) => string,
        None => panic!(
            "could not find `process_outlet` attribute for variant `{}`",
            variant_ident
        ),
    }
}

// Retrieve a vec of `Inlet`s that can be used for code generation from the given struct.
fn inlets(inlets_struct: &syn::ItemStruct, node_attrs: &[syn::Attribute]) -> Vec<Inlet> {
    match inlets_struct.fields {
        syn::Fields::Named(ref fields) => fields
            .named
            .iter()
            .map(|field| {
                let name = field
                    .ident
                    .as_ref()
                    .expect("`inlets` struct field has no ident")
                    .clone();
                let ty = field.ty.clone();
                let process_fn = inlet_process_fn(&node_attrs, &name);
                Inlet {
                    name,
                    ty,
                    process_fn,
                }
            })
            .collect(),
        _ => panic!("`inlets` struct must have named fields"),
    }
}

// Retrieve a vec of `Outlet`s that can be used for code generation from the given enum.
fn outlets(outlets_enum: &syn::ItemEnum, node_attrs: &[syn::Attribute]) -> Vec<Outlet> {
    outlets_enum
        .variants
        .iter()
        .map(|variant| {
            let name = variant.ident.clone();
            let ty = match variant.fields {
                syn::Fields::Unnamed(ref fields) => match fields.unnamed.iter().next() {
                    Some(field) => field.ty.clone(),
                    _ => panic!("expected one unnamed field per `outlets` enum variant"),
                },
                _ => panic!("expected one unnamed field per `outlets` enum variant"),
            };
            let process_fn = outlet_process_fn(&node_attrs, &name);
            Outlet {
                name,
                ty,
                process_fn,
            }
        })
        .collect()
}

/// Generates code necessary for integration within the Gantz Graph as a Node.
///
/// Code generation occurs in the following steps:
///
/// 1. Parse the struct attributes for `inlets` and `outlets` attributes.
/// 2. Parse the `inlets` and `outlets` attributes for the source code for their types.
/// 3. Parse the attr source code for `process_inlet` and `process_outlet` field/variant attrs.
/// 4. Generate remaining necessary private inlet assignment methods.
/// 5. Generate the private `proc_inlet/outlet_at_index` methods.
/// 6. Generate the `node::State` implementation.
/// 7. Produce a TokenStream containing:
///     - `Inlets` struct
///     - `Outlets` enum
///     - Remaining inlet assignment methods.
///     - Proc inlet/outlet at index functions.
///     - `node::State` implementation.
pub fn struct_defs_and_impl_items(
    ast: &syn::DeriveInput,
    ref root: syn::punctuated::Punctuated<syn::PathSegment, syn::token::Colon2>,
) -> (TokenStream, TokenStream) {
    let ident = &ast.ident;
    let (impl_generics, ty_generics, where_clause) = ast.generics.split_for_impl();

    // Retrieve the source string of the inlets attribute if there is one.
    let maybe_inlets_src = inlets_src(&ast.attrs);
    // Retrieve the source string of the outlets attribute if there is one.
    let maybe_outlets_src = outlets_src(&ast.attrs);

    // Parse the inlets src for the inlets struct if there is one.
    let maybe_inlets_struct =
        maybe_inlets_src.map(|src| syn::parse_str::<syn::ItemStruct>(&src).unwrap());
    // Parse the outlets src for the outlets enum if there is one.
    let maybe_outlets_enum =
        maybe_outlets_src.map(|src| syn::parse_str::<syn::ItemEnum>(&src).unwrap());

    // Parse the struct for each inlet.
    let inlets = maybe_inlets_struct
        .as_ref()
        .map(|inlets_struct| inlets(inlets_struct, &ast.attrs))
        .unwrap_or_default();
    let outlets = maybe_outlets_enum
        .as_ref()
        .map(|outlets_enum| outlets(outlets_enum, &ast.attrs))
        .unwrap_or_default();

    // Retrieve the number of inlets and outlets.
    let inlet_count = inlets.iter().count() as u32;
    let outlet_count = outlets.iter().count() as u32;

    // We always require an `inlets` type of some kind in order to satisfy the signature of the
    // `process_outlet` methods. If there is no type, use the unit type.
    let inlets_ty: syn::Type = maybe_inlets_struct
        .as_ref()
        .map(|inlets_struct| {
            let ident = inlets_struct.ident.clone();
            syn::parse(ident.into_token_stream().into()).unwrap()
        })
        .unwrap_or(syn::parse_str("()").unwrap());

    // We always require an `outlets` type of some kind in order to satisfy the `node::Container`
    // signature when performing the `node::State` impl. If there is no type, use the unit type.
    let outlets_ty: syn::Type = maybe_outlets_enum
        .as_ref()
        .map(|outlets_enum| {
            let ident = outlets_enum.ident.clone();
            syn::parse(ident.into_token_stream().into()).unwrap()
        })
        .unwrap_or(syn::parse_str("()").unwrap());

    // Create a name for an inlet assignment method given the index of the inlet.
    fn assign_inlet_method_name(i: u32) -> String {
        format!("assign_inlet_{}", i)
    }

    // Create an ident for an inlet assignment method given the index of the inlet.
    fn assign_inlet_method_ident(i: u32) -> syn::Ident {
        let string = assign_inlet_method_name(i);
        syn::Ident::new(&string, Span::call_site())
    }

    // Generate assignment methods for any inlet fields that do not have a specified
    // `assign_inlet` function. The name of generated methods wil be `process_inlet_x` where `x`
    // is the index of the inlet.
    let remaining_inlet_assignment_methods = inlets
        .iter()
        .enumerate()
        .filter(|(_, inlet)| inlet.process_fn.is_none())
        .map(|(i, inlet)| {
            let i = i as u32;
            let ty = &inlet.ty;
            let assign_fn = assign_inlet_method_ident(i);
            quote! {
                impl #impl_generics #ident #ty_generics #where_clause {
                    fn #assign_fn(&mut self, inlet: &mut #ty, value: &#ty) {
                        *inlet = value.clone();
                    }
                }
            }
        })
        .collect::<Vec<_>>();

    // Generate the method which allows the graph to pass an "incoming" value to the inlet at a
    // specified index.
    let proc_inlet_at_index_method = maybe_inlets_struct.as_ref().map(|_inlets_struct| {
        // An iterator producing an arm for each branch in the match expr.
        let match_arms = inlets.iter().enumerate().map(|(i, inlet)| {
            let i = i as u32;
            let inlet_name = &inlet.name;
            let inlet_ty = &inlet.ty;
            let process_fn = match inlet.process_fn.as_ref() {
                None => {
                    let method_ident = assign_inlet_method_ident(i);
                    quote! { #ident::#method_ident }
                }
                Some(process_fn) => {
                    let method_path: syn::ExprPath = syn::parse_str(&process_fn).unwrap();
                    quote! { #method_path }
                }
            };
            quote! {
                #i => {
                    let incoming_value = incoming
                        .downcast_ref::<#inlet_ty>()
                        .expect("unexpected inlet type");
                    #process_fn(self, &mut inlets.#inlet_name, incoming_value);
                },
            }
        });
        quote! {
            impl #impl_generics #ident #ty_generics #where_clause {
                fn proc_inlet_at_index(
                    &mut self,
                    i: #(#root)*::node::Inlet,
                    inlets: &mut #inlets_ty,
                    incoming: &::std::any::Any,
                ) {
                    match i.0 {
                        #(#match_arms)*
                        ix => panic!("no node inlet for the given index `{}`", ix),
                    }
                }
            }
        }
    });

    // Generate the method which allows the graph to request a value from the outlet at a specified
    // index.
    let proc_outlet_at_index_method = maybe_outlets_enum.as_ref().map(|_outlets_enum| {
        // An iterator producing an arm for each branch in the match expr.
        let match_arms = outlets.iter().enumerate().map(|(i, outlet)| {
            let i = i as u32;
            let outlet_name = &outlet.name;
            let process_fn: syn::ExprPath = syn::parse_str(&outlet.process_fn).unwrap();
            quote! {
                #i => {
                    #outlets_ty::#outlet_name(#process_fn(self, inlets))
                },
            }
        });
        quote! {
            impl #impl_generics #ident #ty_generics #where_clause {
                fn proc_outlet_at_index(
                    &mut self,
                    i: #(#root)*::node::Outlet,
                    inlets: &#inlets_ty,
                ) -> #outlets_ty {
                    match i.0 {
                        #(#match_arms)*
                        ix => panic!("no node outlet for the given index `{}`", ix),
                    }
                }
            }
        }
    });

    // The method used for retrieving the current output value type.
    let outlet_ref_method = maybe_outlets_enum
        .as_ref()
        .map(|_outlets_enum| {
            let match_arms = outlets
                .iter()
                .enumerate()
                .map(|(i, outlet)| {
                    let i = i as u32;
                    let outlet_name = &outlet.name;
                    quote! {
                        (#i, &#outlets_ty::#outlet_name(ref output)) => {
                            output
                        },
                    }
                });
            quote! {
                impl #outlets_ty {
                    fn outlet_ref(&self, i: #(#root)*::node::Outlet) -> &::std::any::Any {
                        match (i.0, self) {
                            #(#match_arms)*
                            (ix, _) => panic!("no value available for outlet at given index `{}`", ix),
                        }
                    }
                }
            }
        })
        .unwrap_or_else(|| {
            quote! {
                impl #outlets_ty {
                    fn outlet_ref(&self, i: #(#root)*::node::Outlet) -> &::std::any::Any {
                        panic!("requested oulet ref but node has no outlets")
                    }
                }
            }
        });

    let node_state_trait_impl = {
        quote! {
            impl #(#root)*::node::State for #(#root)*::node::Container<#inlets_ty, #ident, #outlets_ty> {
                fn n_inlets(&self) -> u32 {
                    #inlet_count
                }

                fn n_outlets(&self) -> u32 {
                    #outlet_count
                }

                fn proc_inlet_at_index(&mut self, i: #(#root)*::node::Inlet, incoming: &::std::any::Any) {
                    let #(#root)*::node::Container { ref mut node, ref mut inlets, .. } = *self;
                    node.proc_inlet_at_index(i, inlets, incoming);
                }

                fn proc_outlet_at_index(&mut self, i: #(#root)*::node::Outlet) -> &::std::any::Any {
                    let #(#root)*::node::Container { ref mut node, ref inlets, ref mut outlet } = *self;
                    *outlet = Some(node.proc_outlet_at_index(i, inlets));
                    outlet.as_ref().unwrap() as _
                }

                fn outlet_ref(&self, i: #(#root)*::node::Outlet) -> &::std::any::Any {
                    match self.outlet {
                        Some(ref outlet) => outlet.outlet_ref(i),
                        None => panic!("no outlet value for node"),
                    }
                }
            }
        }
    };

    let struct_defs = maybe_inlets_struct
        .into_iter()
        .flat_map(|inlets_struct| inlets_struct.into_token_stream())
        .chain({
            maybe_outlets_enum
                .into_iter()
                .flat_map(|outlets_enum| outlets_enum.into_token_stream())
        })
        .collect::<TokenStream>();

    let impl_items = remaining_inlet_assignment_methods
        .into_iter()
        .flat_map(|it| it)
        .chain({ proc_inlet_at_index_method.into_iter().flat_map(|tts| tts) })
        .chain({ proc_outlet_at_index_method.into_iter().flat_map(|tts| tts) })
        .chain(outlet_ref_method.into_iter())
        .chain(node_state_trait_impl)
        .collect::<TokenStream>();

    (struct_defs, impl_items)
}

pub fn impl_gantz_node(ast: &syn::DeriveInput) -> TokenStream {
    let root = {
        let mut path = syn::punctuated::Punctuated::new();
        let path_segment: syn::PathSegment = syn::parse_str("_gantz").unwrap();
        path.push_value(path_segment);
        path
    };
    let (struct_defs, impl_items) = struct_defs_and_impl_items(ast, root);
    let dummy_const_string = format!("_IMPL_GANTZ_NODE_ITEMS_{}", ast.ident);
    let dummy_const = syn::Ident::new(&dummy_const_string, Span::call_site());
    quote! {
        #struct_defs
        #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
        const #dummy_const: () = {
            extern crate gantz as _gantz;
            #impl_items
        };
    }
}

pub fn impl_gantz_node_(ast: &syn::DeriveInput) -> TokenStream {
    let root = Default::default();
    let (struct_defs, impl_items) = struct_defs_and_impl_items(ast, root);
    let dummy_const_string = format!("_IMPL_GANTZ_NODE_ITEMS_{}", ast.ident);
    let dummy_const = syn::Ident::new(&dummy_const_string, Span::call_site());
    quote! {
        #struct_defs
        #[allow(non_upper_case_globals, unused_attributes, unused_qualifications)]
        const #dummy_const: () = {
            #impl_items
        };
    }
}
