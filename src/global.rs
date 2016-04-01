// Copyright 2016 Benoît Labaere (benoit.labaere@gmail.com)
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0>
// or the MIT license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your option.
// This file may not be copied, modified, or distributed except according to those terms.

use std::fmt;
use std::rc::Rc;
use std::cell::Cell;
use std::io::{Error, ErrorKind};
use std::time;

use mio::NotifyError;

/// Defines the socket types, which in turn determines the exact semantics of the socket.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum SocketType {

    /// **One-to-one protocol**  
    /// Pair protocol is the simplest and least scalable scalability protocol. 
    /// It allows scaling by breaking the application in exactly two pieces. 
    /// For example, if a monolithic application handles both accounting and agenda of HR department, 
    /// it can be split into two applications (accounting vs. HR) that are run on two separate servers. 
    /// These applications can then communicate via Pair sockets. 
    /// The downside of this protocol is that its scaling properties are very limited. 
    /// Splitting the application into two pieces allows to scale the two servers. 
    /// To add the third server to the cluster, application has to be split once more, 
    /// say by separating HR functionality into hiring module and salary computation module. 
    /// Whenever possible, try to use one of the more scalable protocols instead.  
    ///  
    /// Socket for communication with exactly one peer.
    /// Each party can send messages at any time. 
    /// If the peer is not available or send buffer is full subsequent calls to [send](struct.Socket.html#method.send) 
    /// will block until it’s possible to send the message.
    Pair       = (    16),

    /// **Publish/subscribe protocol**  
    /// Broadcasts messages to multiple destinations.
    /// Messages are sent from `Pub` sockets and will only be received 
    /// by `Sub` sockets that have subscribed to the matching topic. 
    /// Topic is an arbitrary sequence of bytes at the beginning of the message body. 
    /// The `Sub` socket will determine whether a message should be delivered 
    /// to the user by comparing the subscribed topics to the bytes initial bytes 
    /// in the incomming message, up to the size of the topic.  
    /// Subscribing via [Socket::set_option](struct.Socket.html#method.set_option) and [SocketOption::Subscribe](enum.SocketOption.html#variant.Subscribe)
    /// Will match any message with intial 5 bytes being "Hello", for example, message "Hello, World!" will match.
    /// Topic with zero length matches any message.
    /// If the socket is subscribed to multiple topics, 
    /// message matching any of them will be delivered to the user.
    /// Since the filtering is performed on the Subscriber side, 
    /// all the messages from Publisher will be sent over the transport layer.
    /// The entire message, including the topic, is delivered to the user.  
    ///   
    /// This socket is used to distribute messages to multiple destinations. Receive operation is not defined.
    Pub        = (2 * 16),

    /// Receives messages from the publisher. 
    /// Only messages that the socket is subscribed to are received. 
    /// When the socket is created there are no subscriptions 
    /// and thus no messages will be received. 
    /// Send operation is not defined on this socket.
    Sub        = (2 * 16) + 1,

    /// **Request/reply protocol**  
    /// This protocol is used to distribute the workload among multiple stateless workers.
    /// Please note that request/reply applications should be stateless.
    /// It’s important to include all the information necessary to process the request in the request itself, 
    /// including information about the sender or the originator of the request if this is necessary to respond to the request.
    /// Sender information cannot be retrieved from the underlying socket connection since, 
    /// firstly, transports like IPC may not have a firm notion of a message origin. 
    /// Secondly, transports that have some notion may not have a reliable one 
    /// - a TCP disconnect may mean a new sender, or it may mean a temporary loss in network connectivity.
    /// For this reason, sender information must be included by the application if required. 
    /// Allocating 6 randomly-generated bytes in the message for the lifetime of the connection is sufficient for most purposes. 
    /// For longer-lived applications, an UUID is more suitable.  
    ///   
    /// Used to implement the client application that sends requests and receives replies.
    Req        = (3 * 16),

    /// Used to implement the stateless worker that receives requests and sends replies.
    Rep        = (3 * 16) + 1,

    /// **Pipeline protocol**  
    /// Fair queues messages from the previous processing step and load balances them among instances of the next processing step.  
    ///   
    /// This socket is used to send messages to a cluster of load-balanced nodes. Receive operation is not implemented on this socket type.
    Push       = (5 * 16),

    /// This socket is used to receive a message from a cluster of nodes. Send operation is not implemented on this socket type.
    Pull       = (5 * 16) + 1,

    /// **Survey protocol**  
    /// Allows to broadcast a survey to multiple locations and gather the responses.  
    ///   
    /// Used to send the survey. The survey is delivered to all the connected respondents. 
    /// Once the query is sent, the socket can be used to receive the responses. 
    /// When the survey deadline expires, receive will return ETIMEDOUT error.
    Surveyor   = (6 * 16) + 2,

    /// Use to respond to the survey. 
    /// Survey is received using receive function, response is sent using send function. 
    /// This socket can be connected to at most one peer.
    Respondent = (6 * 16) + 3,

    /// **Message bus protocol**  
    /// Broadcasts messages from any node to all other nodes in the topology. 
    /// The socket should never receives messages that it sent itself.
    /// This pattern scales only to local level (within a single machine or within a single LAN). 
    /// Trying to scale it further can result in overloading individual nodes with messages.  
    /// _Warning_ For bus topology to function correctly, user is responsible for ensuring 
    /// that path from each node to any other node exists within the topology.  
    ///   
    /// Sent messages are distributed to all nodes in the topology. 
    /// Incoming messages from all other nodes in the topology are fair-queued in the socket.
    Bus        = (7 * 16)
}

impl SocketType {
    pub fn id(&self) -> u16 {
        *self as u16
    }

    pub fn peer(&self) -> SocketType {
        match *self {
            SocketType::Pair       => SocketType::Pair,
            SocketType::Pub        => SocketType::Sub,
            SocketType::Sub        => SocketType::Pub,
            SocketType::Req        => SocketType::Rep,
            SocketType::Rep        => SocketType::Req,
            SocketType::Push       => SocketType::Pull,
            SocketType::Pull       => SocketType::Push,
            SocketType::Surveyor   => SocketType::Respondent,
            SocketType::Respondent => SocketType::Surveyor,
            SocketType::Bus        => SocketType::Bus,
        }
    }

    pub fn matches(&self, other: SocketType) -> bool {
        self.peer() == other && other.peer() == *self
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct SocketId(pub usize);

impl fmt::Debug for SocketId {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub struct ProbeId(pub usize);

impl fmt::Debug for ProbeId {
    fn fmt(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone)]
pub struct IdSequence {
    value: Rc<Cell<usize>>
}

impl IdSequence {
    pub fn new() -> IdSequence {
        IdSequence { value: Rc::new(Cell::new(0)) }
    }

    pub fn next(&self) -> usize {
        let id = self.value.get();

        self.value.set(id + 1);
        id
    }
}

impl Default for IdSequence {
    fn default() -> Self {
        IdSequence::new()
    }
}

pub fn other_io_error(msg: &'static str) -> Error {
    Error::new(ErrorKind::Other, msg)
}

pub fn invalid_data_io_error(msg: &'static str) -> Error {
    Error::new(ErrorKind::InvalidData, msg)
}

pub fn would_block_io_error(msg: &'static str) -> Error {
    Error::new(ErrorKind::WouldBlock, msg)
}

pub fn invalid_input_io_error(msg: &'static str) -> Error {
    Error::new(ErrorKind::InvalidInput, msg)
}

pub fn convert_notify_err<T>(err: NotifyError<T>) -> Error {
    match err {
        NotifyError::Io(e) => e,
        NotifyError::Closed(_) => other_io_error("cmd channel closed"),
        NotifyError::Full(_) => Error::new(ErrorKind::WouldBlock, "cmd channel full"),
    }
}

pub trait ToMillis {
    fn to_millis(&self) -> u64;
}

impl ToMillis for time::Duration {
    fn to_millis(&self) -> u64 {
        let millis_from_secs = self.as_secs() * 1_000;
        let millis_from_nanos = self.subsec_nanos() as f64 / 1_000_000f64;

        millis_from_secs + millis_from_nanos as u64
    }
}

#[cfg(test)]
mod tests {
    use super::IdSequence;

    #[test]
    fn id_sequence_can_be_cloned() {
        let seq = IdSequence::new();
        let other = seq.clone();

        assert_eq!(0, other.next());
        assert_eq!(1, seq.next());
        assert_eq!(2, seq.next());
        assert_eq!(3, other.next());
    }
}
