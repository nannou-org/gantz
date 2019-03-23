// Tests for the graph module.

struct One;

struct Add;

struct Debug;

impl gantz::Node for One {
    fn n_inputs(&self) -> u32 {
        0
    }

    fn n_outputs(&self) -> u32 {
        1
    }

    fn expr(&self, args: Vec<syn::Expr>) -> syn::Expr {
        assert!(args.is_empty());
        syn::parse_quote! { 1 }
    }

    fn push_eval(&self) -> Option<gantz::node::PushEval> {
        let item_fn: syn::ItemFn = syn::parse_quote! { fn one() {} };
        Some(item_fn.into())
    }
}

impl gantz::Node for Add {
    fn n_inputs(&self) -> u32 {
        2
    }

    fn n_outputs(&self) -> u32 {
        1
    }

    fn expr(&self, args: Vec<syn::Expr>) -> syn::Expr {
        assert_eq!(args.len(), 2);
        let l = &args[0];
        let r = &args[1];
        syn::parse_quote! { #l + #r }
    }
}

impl gantz::Node for Debug {
    fn n_inputs(&self) -> u32 {
        1
    }

    fn n_outputs(&self) -> u32 {
        0
    }

    fn expr(&self, args: Vec<syn::Expr>) -> syn::Expr {
        assert_eq!(args.len(), 1);
        let input = &args[0];
        syn::parse_quote! { println!("{:?}", #input) }
    }
}

// A simple test graph that adds two "one"s and outputs the result to stdout.
//
//    -------
//    | One |
//    -+-----
//     |\
//     | \
//     |  \
//    -+---+-
//    | Add |
//    -+-----
//     |
//     |
//    -+-----
//    |Debug|
//    -------
#[test]
fn test_graph1() {
    // Instantiate the nodes.
    let one = Box::new(One) as Box<gantz::Node>;
    let add = Box::new(Add) as Box<_>;
    let debug = Box::new(Debug) as Box<_>;

    // Compose the graph.
    let mut g = petgraph::Graph::new();
    let one = g.add_node(one);
    let add = g.add_node(add);
    let debug = g.add_node(debug);
    g.add_edge(one, add, gantz::Edge {
        output: gantz::node::Output(0),
        input: gantz::node::Input(0),
    });
    g.add_edge(one, add, gantz::Edge {
        output: gantz::node::Output(0),
        input: gantz::node::Input(1),
    });
    g.add_edge(add, debug, gantz::Edge {
        output: gantz::node::Output(0),
        input: gantz::node::Input(0),
    });

    // Find all push evaluation enabled nodes. This should just be our `One` node.
    let mut push_ns = gantz::graph::codegen::push_nodes(&g);
    assert_eq!(push_ns.len(), 1);
    let (push_n, fn_decl) = push_ns.pop().unwrap();

    // Generate the push evaluation steps. There should be three, one for each node instance.
    let eval_steps = gantz::graph::codegen::push_eval_steps(&g, push_n);
    assert_eq!(eval_steps.len(), 3);

    // Ensure the order was correct.
    let eval_order: Vec<_> = eval_steps.iter().map(|step| step.node).collect();
    assert_eq!(eval_order, vec![one, add, debug]);

    // Generate the push evaluation function.
    let push_eval_fn = gantz::graph::codegen::push_eval_fn(&g, fn_decl, &eval_steps);
    println!("{}", quote::ToTokens::into_token_stream(push_eval_fn));
}
