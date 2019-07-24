#![recursion_limit = "128"]

extern crate proc_macro;
extern crate proc_macro2;
extern crate syn;
#[macro_use]
extern crate quote;

mod node;

use proc_macro::TokenStream;

#[proc_macro_derive(GantzNode, attributes(inlets, outlets, process_inlet, process_outlet))]
pub fn gantz_node(input: TokenStream) -> TokenStream {
    impl_derive(input, node::impl_gantz_node)
}

#[proc_macro_derive(GantzNode_, attributes(inlets, outlets, process_inlet, process_outlet))]
pub fn gantz_node_(input: TokenStream) -> TokenStream {
    impl_derive(input, node::impl_gantz_node_)
}

// Use the given function to generate a TokenStream for the derive implementation.
fn impl_derive(
    input: TokenStream,
    generate_derive: fn(&syn::DeriveInput) -> proc_macro2::TokenStream,
) -> TokenStream {
    // Parse the input TokenStream representation.
    let ast = syn::parse(input).unwrap();
    // Build the implementation.
    let gen = generate_derive(&ast);
    // Return the generated impl.
    gen.into()
}
