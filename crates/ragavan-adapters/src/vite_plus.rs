use crate::{Invocation, Stack, vite};

pub(super) const ADAPTER: Stack = Stack {
    recognize,
    adjust: vite::adjust,
};

fn recognize(invocation: &Invocation) -> bool {
    invocation.invokes("vp")
        && invocation
            .arguments()
            .first()
            .is_some_and(|argument| argument == "dev")
}
