//! Ensure that its possible to serialize/deserialize core nodes as trait
//! objects using typetag.

/// A wrapper around the **Node** trait that allows for serializing and
/// deserializing node trait objects.
#[typetag::serde(tag = "type")]
trait SerdeNode: Node {}

#[typetag::serde]
impl SerdeNode for node::Expr {}
#[typetag::serde]
impl SerdeNode for node::Push<node::Expr> {}
#[typetag::serde]
impl SerdeNode for node::Pull<node::Expr> {}

use gantz_core::node::{self, Expr, Node, Pull, Push, WithPullEval, WithPushEval};
use serde_json;

// Helper function to create a basic expression node
fn basic_expr() -> Expr {
    node::expr("(+ $a $b)").unwrap()
}

// Helper function to create a pushable expression node
fn push_expr() -> Push<Expr> {
    node::expr("(+ $a $b)").unwrap().with_push_eval()
}

// Helper function to create a pullable expression node
fn pull_expr() -> Pull<Expr> {
    node::expr("(+ $a $b)").unwrap().with_pull_eval()
}

// Test serializing and deserializing a basic Expr node
#[test]
fn test_serde_basic_expr() {
    let node = basic_expr();

    // Create a boxed SerdeNode
    let boxed: Box<dyn SerdeNode> = Box::new(node);

    // Serialize to JSON
    let serialized = serde_json::to_string(&boxed).expect("Failed to serialize");

    // Deserialize from JSON
    let deserialized: Box<dyn SerdeNode> =
        serde_json::from_str(&serialized).expect("Failed to deserialize");

    // Check properties
    let n = deserialized;
    assert_eq!(n.n_inputs(), 2);
    assert_eq!(n.n_outputs(), 1);
    assert!(n.push_eval().is_empty());
    assert!(n.pull_eval().is_empty());
}

// Test serializing and deserializing a Push node
#[test]
fn test_serde_push_node() {
    let node = push_expr();

    // Create a boxed SerdeNode
    let boxed: Box<dyn SerdeNode> = Box::new(node);

    // Serialize to JSON
    let serialized = serde_json::to_string(&boxed).expect("Failed to serialize");

    // Deserialize from JSON
    let deserialized: Box<dyn SerdeNode> =
        serde_json::from_str(&serialized).expect("Failed to deserialize");

    // Check properties
    let n = deserialized;
    assert_eq!(n.n_inputs(), 2);
    assert_eq!(n.n_outputs(), 1);
    assert!(!n.push_eval().is_empty());
    assert!(n.pull_eval().is_empty());
}

// Test serializing and deserializing a vector of various node types
#[test]
fn test_serde_node_vector() {
    // Create a vector of different node types
    let nodes: Vec<Box<dyn SerdeNode>> = vec![
        Box::new(basic_expr()),
        Box::new(push_expr()),
        Box::new(pull_expr()),
    ];

    // Serialize the vector
    let serialized = serde_json::to_string(&nodes).expect("Failed to serialize vector");

    // Deserialize the vector
    let deserialized: Vec<Box<dyn SerdeNode>> =
        serde_json::from_str(&serialized).expect("Failed to deserialize vector");

    // Check count
    assert_eq!(nodes.len(), deserialized.len());

    // First node should be basic expr
    assert!(deserialized[0].push_eval().is_empty());
    assert!(deserialized[0].pull_eval().is_empty());

    // Second node should be push node
    assert!(!deserialized[1].push_eval().is_empty());
    assert!(deserialized[1].pull_eval().is_empty());

    // Third node should be pull node
    assert!(deserialized[2].push_eval().is_empty());
    assert!(!deserialized[2].pull_eval().is_empty());
}
