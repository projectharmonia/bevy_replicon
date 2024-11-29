use std::ops::Range;

/// Component insertions or mutations for an entity in form of serialized ranges
/// from [`SerializedData`](super::serialized_data::SerializedData).
///
/// Used inside [`ChangeMessage`](super::change_message::ChangeMessage) and
/// [`MutateMessage`](super::mutate_message::MutateMessage).
pub(super) struct ComponentChanges {
    pub(super) entity: Range<usize>,
    pub(super) components_len: usize,
    pub(super) components: Vec<Range<usize>>,
}

impl ComponentChanges {
    pub(super) fn components_size(&self) -> usize {
        self.components
            .iter()
            .map(|range| range.len())
            .sum::<usize>()
    }

    pub(super) fn add_component(&mut self, component: Range<usize>) {
        self.components_len += 1;

        if let Some(last) = self.components.last_mut() {
            // Append to previous range if possible.
            if last.end == component.start {
                last.end = component.end;
                return;
            }
        }

        self.components.push(component);
    }

    pub(super) fn extend(&mut self, other: &Self) {
        self.components.extend(other.components.iter().cloned());
        self.components_len += other.components_len;
    }
}
