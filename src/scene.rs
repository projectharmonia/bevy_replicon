use bevy::{ecs::archetype::ArchetypeId, prelude::*, scene::DynamicEntity};

use crate::replicon_core::replication_rules::ReplicationRules;

/**
Fills scene with all replicated entities and their components.

Entities won't have the [`Replication`](crate::replicon_core::replication_rules::Replication) component.
So on deserialization you need to insert it back if you want entities to continue to replicate.

# Panics

Panics if any replicated component is not registered using `register_type()`
or `#[reflect(Component)]` is missing.

# Examples

```
use bevy::{prelude::*, scene::serde::SceneDeserializer};
use bevy_replicon::{prelude::*, scene};
use serde::de::DeserializeSeed;
# let mut world = World::new();
# world.init_resource::<AppTypeRegistry>();
# world.init_resource::<ReplicationRules>();

// Serialization
let registry = world.resource::<AppTypeRegistry>();
let mut scene = DynamicScene::default();
scene::replicate_into(&mut scene, &world);
let scene = scene
    .serialize_ron(&registry)
    .expect("scene should be serialized");

// Deserialization
let scene_deserializer = SceneDeserializer {
    type_registry: &registry.read(),
};
let mut deserializer =
    ron::Deserializer::from_str(&scene).expect("scene should be serialized as valid ron");
let mut scene = scene_deserializer
    .deserialize(&mut deserializer)
    .expect("ron should be convertible to scene");

// All saved entities should have `Replication` component.
for entity in &mut scene.entities {
    entity.components.push(Replication.clone_value());
}
```
*/
pub fn replicate_into(scene: &mut DynamicScene, world: &World) {
    let registry = world.resource::<AppTypeRegistry>();
    let replication_rules = world.resource::<ReplicationRules>();

    let registry = registry.read();
    for archetype in world
        .archetypes()
        .iter()
        .filter(|archetype| archetype.id() != ArchetypeId::EMPTY)
        .filter(|archetype| archetype.id() != ArchetypeId::INVALID)
        .filter(|archetype| archetype.contains(replication_rules.get_marker_id()))
    {
        let entities_offset = scene.entities.len();
        for archetype_entity in archetype.entities() {
            scene.entities.push(DynamicEntity {
                entity: archetype_entity.entity(),
                components: Vec::new(),
            });
        }

        for component_id in archetype.components() {
            let Some((_, replication_info)) = replication_rules.get(component_id) else {
                continue;
            };
            if archetype.contains(replication_info.ignored_id) {
                continue;
            }

            // SAFETY: `component_info` obtained from the world.
            let component_info = unsafe { world.components().get_info_unchecked(component_id) };
            let type_name = component_info.name();
            let type_id = component_info
                .type_id()
                .unwrap_or_else(|| panic!("{type_name} should have registered TypeId"));
            let registration = registry
                .get(type_id)
                .unwrap_or_else(|| panic!("{type_name} should be registered"));
            let reflect_component = registration
                .data::<ReflectComponent>()
                .unwrap_or_else(|| panic!("{type_name} should have reflect(Component)"));

            for (index, archetype_entity) in archetype.entities().iter().enumerate() {
                let component = reflect_component
                    .reflect(world.entity(archetype_entity.entity()))
                    .unwrap_or_else(|| panic!("entity should have {type_name}"));

                scene.entities[entities_offset + index]
                    .components
                    .push(component.clone_value());
            }
        }
    }
}
