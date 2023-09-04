use std::{
    any,
    fmt::{self, Formatter},
    net::{Ipv4Addr, SocketAddr, UdpSocket},
    time::SystemTime,
};

use bevy::{
    ecs::entity::EntityMap,
    prelude::*,
    reflect::{
        serde::{ReflectSerializer, UntypedReflectDeserializer},
        TypeRegistryInternal,
    },
};
use bevy_renet::renet::{
    transport::{
        ClientAuthentication, NetcodeClientTransport, NetcodeServerTransport, ServerAuthentication,
        ServerConfig,
    },
    ChannelConfig, ConnectionConfig, RenetClient, RenetServer,
};
use bevy_replicon::prelude::*;
use serde::{
    de::{self, DeserializeSeed, SeqAccess, Visitor},
    ser::SerializeStruct,
    Deserialize, Deserializer, Serialize, Serializer,
};
use strum::{EnumVariantNames, IntoStaticStr, VariantNames};

pub(super) fn setup(server_app: &mut App, client_app: &mut App) {
    let server_channels = server_app
        .world
        .resource_mut::<NetworkChannels>()
        .server_channels();
    let client_channels = client_app
        .world
        .resource_mut::<NetworkChannels>()
        .client_channels();

    let (server, server_transport) =
        create_server(server_channels.clone(), client_channels.clone());
    let (client, client_transport) = create_client(
        server_transport.addr().port(),
        server_channels,
        client_channels,
    );

    server_app
        .insert_resource(server)
        .insert_resource(server_transport);

    client_app
        .insert_resource(client)
        .insert_resource(client_transport);

    loop {
        client_app.update();
        server_app.update();
        if client_app
            .world
            .resource::<NetcodeClientTransport>()
            .is_connected()
        {
            break;
        }
    }
}

const PROTOCOL_ID: u64 = 0;

fn create_server(
    server_channels_config: Vec<ChannelConfig>,
    client_channels_config: Vec<ChannelConfig>,
) -> (RenetServer, NetcodeServerTransport) {
    let server = RenetServer::new(ConnectionConfig {
        server_channels_config,
        client_channels_config,
        ..Default::default()
    });

    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let server_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0);
    let socket = UdpSocket::bind(server_addr).expect("localhost should be bindable");
    let public_addr = socket
        .local_addr()
        .expect("socket should autodetect local address");
    let server_config = ServerConfig {
        max_clients: 1,
        protocol_id: PROTOCOL_ID,
        public_addr,
        authentication: ServerAuthentication::Unsecure,
    };

    let transport = NetcodeServerTransport::new(current_time, server_config, socket).unwrap();

    (server, transport)
}

fn create_client(
    port: u16,
    server_channels_config: Vec<ChannelConfig>,
    client_channels_config: Vec<ChannelConfig>,
) -> (RenetClient, NetcodeClientTransport) {
    let client = RenetClient::new(ConnectionConfig {
        server_channels_config,
        client_channels_config,
        ..Default::default()
    });

    let current_time = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    let client_id = current_time.as_millis() as u64;
    let ip = Ipv4Addr::LOCALHOST.into();
    let server_addr = SocketAddr::new(ip, port);
    let socket = UdpSocket::bind((ip, 0)).expect("localhost should be bindable");
    let authentication = ClientAuthentication::Unsecure {
        client_id,
        protocol_id: PROTOCOL_ID,
        server_addr,
        user_data: None,
    };
    let transport = NetcodeClientTransport::new(current_time, authentication, socket).unwrap();

    (client, transport)
}

#[derive(Reflect, Debug)]
pub(super) struct ReflectEventComponent;

#[derive(Debug, Deserialize, Event, Serialize)]
pub(super) struct DummyEvent(pub(super) Entity);

impl MapEventEntities for DummyEvent {
    fn map_entities(&mut self, entity_map: &EntityMap) -> Result<(), MapError> {
        self.0 = entity_map.get(self.0).ok_or(MapError(self.0))?;
        Ok(())
    }
}

#[derive(Debug, Event)]
pub(super) struct ReflectEvent {
    pub(super) entity: Entity,
    pub(super) component: Box<dyn Reflect>,
}

impl MapEventEntities for ReflectEvent {
    fn map_entities(&mut self, entity_map: &EntityMap) -> Result<(), MapError> {
        self.entity = entity_map.get(self.entity).ok_or(MapError(self.entity))?;
        Ok(())
    }
}

#[derive(IntoStaticStr, EnumVariantNames)]
#[strum(serialize_all = "snake_case")]
enum ReflectEventField {
    Entity,
    Component,
}

pub(super) struct ReflectEventSerializer<'a> {
    registry: &'a TypeRegistryInternal,
    event: &'a ReflectEvent,
}

impl BuildEventSerializer<ReflectEvent> for ReflectEventSerializer<'_> {
    type EventSerializer<'a> = ReflectEventSerializer<'a>;

    fn new<'a>(
        event: &'a ReflectEvent,
        registry: &'a TypeRegistryInternal,
    ) -> Self::EventSerializer<'a> {
        Self::EventSerializer { event, registry }
    }
}

impl Serialize for ReflectEventSerializer<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct(
            any::type_name::<ReflectEvent>(),
            ReflectEventField::VARIANTS.len(),
        )?;
        state.serialize_field(ReflectEventField::Entity.into(), &self.event.entity)?;
        state.serialize_field(
            ReflectEventField::Entity.into(),
            &ReflectSerializer::new(&*self.event.component, self.registry),
        )?;
        state.end()
    }
}

pub(super) struct ReflectEventDeserializer<'a> {
    registry: &'a TypeRegistryInternal,
}

impl BuildEventDeserializer for ReflectEventDeserializer<'_> {
    type EventDeserializer<'a> = ReflectEventDeserializer<'a>;

    fn new(registry: &TypeRegistryInternal) -> Self::EventDeserializer<'_> {
        Self::EventDeserializer { registry }
    }
}

impl<'de> DeserializeSeed<'de> for ReflectEventDeserializer<'_> {
    type Value = ReflectEvent;

    fn deserialize<D: Deserializer<'de>>(self, deserializer: D) -> Result<Self::Value, D::Error> {
        deserializer.deserialize_struct(
            any::type_name::<Self::Value>(),
            ReflectEventField::VARIANTS,
            self,
        )
    }
}

impl<'de> Visitor<'de> for ReflectEventDeserializer<'_> {
    type Value = ReflectEvent;

    fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
        formatter.write_str(any::type_name::<Self::Value>())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        let entity = seq
            .next_element()?
            .ok_or_else(|| de::Error::invalid_length(ReflectEventField::Entity as usize, &self))?;
        let component = seq
            .next_element_seed(UntypedReflectDeserializer::new(self.registry))?
            .ok_or_else(|| {
                de::Error::invalid_length(ReflectEventField::Component as usize, &self)
            })?;
        Ok(ReflectEvent { entity, component })
    }
}
