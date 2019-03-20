fn main() {
    let path = format!("{}/examples/foo", env!("CARGO_MANIFEST_DIR"));
    let project = gantz::Project::open(path.into()).unwrap();

    // project.root_node(

    // // Add nodes to the
    // project.add_node(
}
