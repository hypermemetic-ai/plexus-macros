//! RETURNS-46 negative fixture: methods and children share one route
//! namespace, and so one aggregate. A method whose wire name is a child's
//! route segment would put two variants under one tag (`"branch"`): the
//! macro refuses it, naming both.

use futures::Stream;

#[derive(Clone)]
pub struct TreeActivation;

#[plexus_macros::activation(crate_path = "plexus_core")]
impl TreeActivation {
    /// Renamed onto the child's segment.
    #[plexus_macros::method(name = "branch")]
    async fn grow(&self) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::empty()
    }

    /// The child.
    #[plexus_macros::child]
    fn branch(&self) -> BranchActivation {
        BranchActivation
    }
}

#[derive(Clone)]
pub struct BranchActivation;

#[plexus_macros::activation(crate_path = "plexus_core")]
impl BranchActivation {
    /// A leaf method.
    #[plexus_macros::method]
    async fn bud(&self) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::empty()
    }
}

fn main() {}
