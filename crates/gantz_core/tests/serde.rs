//! Tests for SerdeNode serialization and deserialization.

use gantz_core::node::{self, Expr, Pull, Push, SerdeNode, WithPullEval, WithPushEval};
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
    let n = deserialized.node();
    assert_eq!(n.n_inputs(), 2);
    assert_eq!(n.n_outputs(), 1);
    assert!(n.push_eval().is_none());
    assert!(n.pull_eval().is_none());

    // Check expression result format
    let expr = n.expr(&[None, None]);
    assert_eq!(format!("{}", expr), "(+ (quote ()) (quote ()))");
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
    let n = deserialized.node();
    assert_eq!(n.n_inputs(), 2);
    assert_eq!(n.n_outputs(), 1);
    assert!(n.push_eval().is_some());
    assert!(n.pull_eval().is_none());

    // Check expression result format
    let expr = n.expr(&[None, None]);
    assert_eq!(format!("{}", expr), "(+ (quote ()) (quote ()))");
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
    assert!(deserialized[0].node().push_eval().is_none());
    assert!(deserialized[0].node().pull_eval().is_none());

    // Second node should be push node
    assert!(deserialized[1].node().push_eval().is_some());
    assert!(deserialized[1].node().pull_eval().is_none());

    // Third node should be pull node
    assert!(deserialized[2].node().push_eval().is_none());
    assert!(deserialized[2].node().pull_eval().is_some());
}
