use bevy::{
    prelude::*,
    utils::{EntityHashMap, EntityHashSet},
};

use super::VisibilityPolicy;

/// Entity visibility settings for a client.
pub struct ClientVisibility {
    filter: VisibilityFilter,
    entity_state: Visibility,
}

impl ClientVisibility {
    /// Creates a new instance based on the preconfigured policy.
    pub(super) fn new(policy: VisibilityPolicy) -> Self {
        match policy {
            VisibilityPolicy::All => Self::with_filter(VisibilityFilter::All {
                just_connected: true,
            }),
            VisibilityPolicy::Blacklist => Self::with_filter(VisibilityFilter::Blacklist {
                list: Default::default(),
                added: Default::default(),
                removed: Default::default(),
            }),
            VisibilityPolicy::Whitelist => Self::with_filter(VisibilityFilter::Whitelist {
                list: Default::default(),
                added: Default::default(),
                removed: Default::default(),
            }),
        }
    }

    /// Creates a new instance with a specific filter.
    fn with_filter(filter: VisibilityFilter) -> Self {
        Self {
            filter,
            entity_state: Default::default(),
        }
    }

    /// Resets the filter state to as it was after [`Self::new`].
    ///
    /// `entity_state` remains untouched.
    pub(super) fn clear(&mut self) {
        match &mut self.filter {
            VisibilityFilter::All { just_connected } => *just_connected = true,
            VisibilityFilter::Blacklist {
                list,
                added,
                removed,
            }
            | VisibilityFilter::Whitelist {
                list,
                added,
                removed,
            } => {
                list.clear();
                added.clear();
                removed.clear();
            }
        }
    }

    /// Marks all entities as not "just added" and clears the removed.
    ///
    /// Should be called after each tick.
    pub(crate) fn update(&mut self) {
        match &mut self.filter {
            VisibilityFilter::All { just_connected } => *just_connected = false,
            VisibilityFilter::Blacklist {
                list,
                added,
                removed,
            } => {
                for entity in removed.drain() {
                    list.remove(&entity);
                }
                added.clear();
            }
            VisibilityFilter::Whitelist {
                list,
                added,
                removed,
            } => {
                for entity in added.drain() {
                    list.insert(entity, false);
                }
                removed.clear();
            }
        }
    }

    /// Removes a despawned entity tracked by this client.
    pub(super) fn remove_despawned(&mut self, entity: Entity) {
        match &mut self.filter {
            VisibilityFilter::All { .. } => (),
            VisibilityFilter::Blacklist {
                list,
                added,
                removed,
            } => {
                if let Some(just_removed) = list.get_mut(&entity) {
                    *just_removed = true;
                    removed.remove(&entity);
                    added.remove(&entity);
                }
            }
            VisibilityFilter::Whitelist {
                list,
                added,
                removed,
            } => {
                if list.remove(&entity).is_some() {
                    added.remove(&entity);
                    removed.remove(&entity);
                }
            }
        }
    }

    /// Returns an iterator of entities the client lost visibility of this tick.
    pub(super) fn iter_lost_visibility(&self) -> impl Iterator<Item = Entity> + '_ {
        match &self.filter {
            VisibilityFilter::All { .. } => VisibilityLostIter::AllVisible,
            VisibilityFilter::Blacklist { added, .. } => {
                VisibilityLostIter::Lost(added.iter().copied())
            }
            VisibilityFilter::Whitelist { removed, .. } => {
                VisibilityLostIter::Lost(removed.iter().copied())
            }
        }
    }

    /// Sets visibility for a specific entity.
    ///
    /// Does nothing if the visibility policy for the server plugin is set to [`VisibilityPolicy::All`].
    pub fn set_visibility(&mut self, entity: Entity, visibile: bool) {
        match &mut self.filter {
            VisibilityFilter::All { .. } => {
                if visibile {
                    debug!(
                        "ignoring visibility enable due to {:?}",
                        VisibilityPolicy::All
                    );
                } else {
                    warn!(
                        "ignoring visibility disable due to {:?}",
                        VisibilityPolicy::All
                    );
                }
            }
            VisibilityFilter::Blacklist {
                list,
                added,
                removed,
            } => {
                if !visibile {
                    if list.insert(entity, false).is_none() {
                        removed.remove(&entity);
                        added.insert(entity);
                    }
                } else if let Some(just_removed) = list.get_mut(&entity) {
                    *just_removed = true;
                    removed.insert(entity);
                    added.remove(&entity);
                }
            }
            VisibilityFilter::Whitelist {
                list,
                added,
                removed,
            } => {
                if visibile {
                    if list.insert(entity, true).is_none() {
                        removed.remove(&entity);
                        added.insert(entity);
                    }
                } else if list.remove(&entity).is_some() {
                    removed.insert(entity);
                    added.remove(&entity);
                }
            }
        }
    }

    /// Gets visibility for a specific entity.
    pub fn is_visible(&self, entity: Entity) -> bool {
        match &self.filter {
            VisibilityFilter::All { .. } => true,
            VisibilityFilter::Blacklist { list, .. } => !list.contains_key(&entity),
            VisibilityFilter::Whitelist { list, .. } => list.contains_key(&entity),
        }
    }

    /// Reads entity visibility state for specific entity.
    ///
    /// Can be obtained later from [`Self::entity_state`].
    pub(crate) fn read_entity_state(&mut self, entity: Entity) {
        match &mut self.filter {
            VisibilityFilter::All { just_connected } => {
                if *just_connected {
                    self.entity_state = Visibility::Gained
                } else {
                    self.entity_state = Visibility::Visible
                }
            }
            VisibilityFilter::Blacklist { list, .. } => match list.get(&entity) {
                Some(true) => self.entity_state = Visibility::Gained,
                Some(false) => self.entity_state = Visibility::Hidden,
                None => self.entity_state = Visibility::Visible,
            },
            VisibilityFilter::Whitelist { list, .. } => match list.get(&entity) {
                Some(true) => self.entity_state = Visibility::Gained,
                Some(false) => self.entity_state = Visibility::Visible,
                None => self.entity_state = Visibility::Hidden,
            },
        }
    }

    /// Returns state obtained from last call of [`Self::read_entity_state`].
    pub(crate) fn entity_state(&self) -> Visibility {
        self.entity_state
    }
}

/// Filter for [`ClientVisibility`] based on [`VisibilityPolicy`].
enum VisibilityFilter {
    All {
        /// Indicates that the client has just connected to the server.
        just_connected: bool,
    },
    Blacklist {
        /// All blacklisted entities and an indicator of whether it is in the queue for deletion
        /// at the end of this tick.
        list: EntityHashMap<Entity, bool>,
        /// All entities that were removed from the list in this tick.
        added: EntityHashSet<Entity>,
        /// All entities that were added to the list in this tick.
        removed: EntityHashSet<Entity>,
    },
    Whitelist {
        /// All whitelisted entities and an indicator whether it was added to the list in this tick.
        list: EntityHashMap<Entity, bool>,
        /// All entities that were added to the list in this tick.
        added: EntityHashSet<Entity>,
        /// All entities that were removed from the list in this tick.
        removed: EntityHashSet<Entity>,
    },
}

/// Visibility state for an entity in the current tick, from the perspective of one client.
///
/// Note that the distinction between 'lost visibility' and 'don't have visibility' is not exposed here.
/// There is only `Visibility::Hidden` to encompass both variants.
///
/// Lost visibility is handled separately with [`ClientVisibility::iter_lost_visibility`].
#[derive(PartialEq, Default, Clone, Copy)]
pub(crate) enum Visibility {
    /// The client does not have visibility of the entity in this tick.
    #[default]
    Hidden,
    /// The client gained visibility of the entity in this tick (it was not visible in the previous tick).
    Gained,
    /// The entity is visible to the client (and was visible in the previous tick).
    Visible,
}

enum VisibilityLostIter<T> {
    AllVisible,
    Lost(T),
}

impl<T: Iterator> Iterator for VisibilityLostIter<T> {
    type Item = T::Item;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            VisibilityLostIter::AllVisible => None,
            VisibilityLostIter::Lost(entities) => entities.next(),
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        match self {
            VisibilityLostIter::AllVisible => (0, Some(0)),
            VisibilityLostIter::Lost(entities) => entities.size_hint(),
        }
    }
}

impl<T: ExactSizeIterator> ExactSizeIterator for VisibilityLostIter<T> {}
