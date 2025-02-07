use serde::{Deserialize, Serialize};

/// Identifier for mutate messages.
///
/// Use for mutations acknowledgement.
///
/// Its serialization uses fixint encoding as serializing ticks as varints increases the average message size.
/// A tick >= 2^14 will be [5 bytes](https://postcard.jamesmunns.com/wire-format.html#maximum-encoded-length)
/// At 60 ticks/sec, that will happen after ~5 minutes. So any session over this time period would transmit
/// more total bytes with varint encoding.
#[derive(Default, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct MutateIndex(#[serde(with = "postcard::fixint::le")] u16);

impl MutateIndex {
    /// Returns the current value and increments `self` by 1.
    ///
    /// Wraps on overflow.
    pub(crate) fn advance(&mut self) -> Self {
        let next = *self;
        self.0 = self.0.wrapping_add(1);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advance() {
        let mut index = MutateIndex(u16::MAX - 1);

        assert_eq!(index.advance(), MutateIndex(u16::MAX - 1));
        assert_eq!(index, MutateIndex(u16::MAX));

        assert_eq!(index.advance(), MutateIndex(u16::MAX));
        assert_eq!(index, MutateIndex(0));
    }
}
