use super::adjust;
use crate::{script::Invocation, stacks::Stack};

pub(in crate::stacks) const ADAPTER: Stack = Stack { recognize, adjust };

fn recognize(invocation: &Invocation) -> bool {
    invocation.invokes("vp")
        && invocation
            .arguments()
            .first()
            .is_some_and(|argument| argument == "dev")
}
