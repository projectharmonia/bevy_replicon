//! Wrappers around renet types to provide integration with Bevy.
//! We don't use `bevy` feature from `renet` because we don't want to depend on renet releases.

use std::{net::UdpSocket, time::Duration};

use bevy::prelude::*;
#[cfg(feature = "renet_transport")]
use renet::transport::{NetcodeError, ServerConfig};
use renet::{transport::ClientAuthentication, ClientId, ConnectionConfig, DisconnectReason};

/// Wrapper around [`renet::RenetClient`] to make it a resource.
#[derive(Debug, Resource, Deref, DerefMut)]
pub struct RenetClient(renet::RenetClient);

impl RenetClient {
    pub fn new(config: ConnectionConfig) -> Self {
        Self(renet::RenetClient::new(config))
    }
}

/// Wrapper around [`renet::RenetServer`] to make it a resource.
#[derive(Debug, Resource, Deref, DerefMut)]
pub struct RenetServer(renet::RenetServer);

impl RenetServer {
    pub fn new(connection_config: ConnectionConfig) -> Self {
        Self(renet::RenetServer::new(connection_config))
    }
}

/// Copies [`renet::ServerEvent`] to pass it in events.
#[derive(Debug, PartialEq, Eq, Event)]
pub enum ServerEvent {
    ClientConnected {
        client_id: ClientId,
    },
    ClientDisconnected {
        client_id: ClientId,
        reason: DisconnectReason,
    },
}

impl From<renet::ServerEvent> for ServerEvent {
    fn from(value: renet::ServerEvent) -> Self {
        match value {
            renet::ServerEvent::ClientConnected { client_id } => {
                Self::ClientConnected { client_id }
            }
            renet::ServerEvent::ClientDisconnected { client_id, reason } => {
                Self::ClientDisconnected { client_id, reason }
            }
        }
    }
}

/// Wrapper around [`renet::transport::NetcodeClientTransport`] to make it a resource.
#[cfg(feature = "renet_transport")]
#[derive(Debug, Resource, Deref, DerefMut)]
pub struct NetcodeClientTransport(renet::transport::NetcodeClientTransport);

impl NetcodeClientTransport {
    pub fn new(
        current_time: Duration,
        authentication: ClientAuthentication,
        socket: UdpSocket,
    ) -> Result<Self, NetcodeError> {
        Ok(Self(renet::transport::NetcodeClientTransport::new(
            current_time,
            authentication,
            socket,
        )?))
    }
}

/// Wrapper around [`renet::transport::NetcodeServerTransport`] to make it a resource.
#[cfg(feature = "renet_transport")]
#[derive(Debug, Resource, Deref, DerefMut)]
pub struct NetcodeServerTransport(renet::transport::NetcodeServerTransport);

impl NetcodeServerTransport {
    pub fn new(server_config: ServerConfig, socket: UdpSocket) -> Result<Self, std::io::Error> {
        Ok(Self(renet::transport::NetcodeServerTransport::new(
            server_config,
            socket,
        )?))
    }
}

/// Wrapper around [`renet::transport::NetcodeTransportError`] to pass it in events.
#[cfg(feature = "renet_transport")]
#[derive(Debug, Event)]
pub enum NetcodeTransportError {
    Netcode(NetcodeError),
    Renet(DisconnectReason),
    IO(std::io::Error),
}

impl From<renet::transport::NetcodeTransportError> for NetcodeTransportError {
    fn from(value: renet::transport::NetcodeTransportError) -> Self {
        match value {
            renet::transport::NetcodeTransportError::Netcode(error) => Self::Netcode(error),
            renet::transport::NetcodeTransportError::Renet(error) => Self::Renet(error),
            renet::transport::NetcodeTransportError::IO(error) => Self::IO(error),
        }
    }
}
