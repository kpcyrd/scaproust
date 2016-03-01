// Copyright 2016 Benoît Labaere (benoit.labaere@gmail.com)
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0>
// or the MIT license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your option.
// This file may not be copied, modified, or distributed except according to those terms.

use std::io;
use std::sync::mpsc::Receiver;
use std::time;

use mio;
use mio::Sender;

use global::*;
use event_loop_msg::*;
use Message;
use endpoint_facade::EndpointFacade;

pub struct SocketFacade {
    id: SocketId,
    socket_type: SocketType, 
    cmd_sender: Sender<EventLoopSignal>,
    evt_receiver: Receiver<SocketNotify>
    // Could use https://github.com/polyfractal/bounded-spsc-queue ?
    // Maybe once a smart waiting strategy is available (like spin, then sleep 0, then sleep 1, then mutex ?)
    // or something that would help for poll
}

impl SocketFacade {

    #[doc(hidden)]
    pub fn new(
        id: SocketId,
        socket_type: SocketType, 
        cmd_tx: Sender<EventLoopSignal>, 
        evt_rx: Receiver<SocketNotify>) -> SocketFacade {
        SocketFacade { 
            id: id, 
            socket_type: socket_type,
            cmd_sender: cmd_tx, 
            evt_receiver: evt_rx 
        }
    }

    #[doc(hidden)]
    pub fn get_id(&self) -> SocketId {
        self.id
    }

    #[doc(hidden)]
    pub fn get_socket_type(&self) -> SocketType {
        self.socket_type
    }

    fn send_cmd(&self, cmd: SocketCmdSignal) -> Result<(), io::Error> {
        let cmd_sig = CmdSignal::Socket(self.id, cmd);
        let loop_sig = EventLoopSignal::Cmd(cmd_sig);

        self.cmd_sender.send(loop_sig).map_err(|e| convert_notify_err(e))
    }

    /// Adds a remote endpoint to the socket.
    /// The library would then try to connect to the specified remote endpoint.
    /// The transport specifies the underlying transport protocol to use.
    /// The meaning of the address part is specific to the underlying transport protocol.
    /// Note that bind and connect may be called multiple times on the same socket,
    /// thus allowing the socket to communicate with multiple heterogeneous endpoints.
    /// On success, returns an [Endpoint](struct.Endpoint.html) that can be later used to remove the endpoint from the socket.
    pub fn connect(&mut self, addr: &str) -> Result<EndpointFacade, io::Error> {
        let cmd = SocketCmdSignal::Connect(addr.to_owned());
        
        try!(self.send_cmd(cmd));

        match self.evt_receiver.recv() {
            Ok(SocketNotify::Connected(t))    => Ok(self.new_endpoint(t)),
            Ok(SocketNotify::NotConnected(e)) => Err(e),
            Ok(_)                             => Err(other_io_error("unexpected evt")),
            Err(_)                            => Err(other_io_error("evt channel closed"))
        }
    }

    /// Adds a local endpoint to the socket. The endpoint can be then used by other applications to connect to.
    /// The addr argument consists of two parts as follows: transport://address.
    /// The transport specifies the underlying transport protocol to use.
    /// The meaning of the address part is specific to the underlying transport protocol.
    /// Note that bind and connect may be called multiple times on the same socket,
    /// thus allowing the socket to communicate with multiple heterogeneous endpoints.
    /// On success, returns an [Endpoint](struct.Endpoint.html) that can be later used to remove the endpoint from the socket.
    pub fn bind(&mut self, addr: &str) -> Result<EndpointFacade, io::Error> {
        let cmd = SocketCmdSignal::Bind(addr.to_owned());
        
        try!(self.send_cmd(cmd));

        match self.evt_receiver.recv() {
            Ok(SocketNotify::Bound(t))    => Ok(self.new_endpoint(t)),
            Ok(SocketNotify::NotBound(e)) => Err(e),
            Ok(_)                         => Err(other_io_error("unexpected evt")),
            Err(_)                        => Err(other_io_error("evt channel closed"))
        }
    }

    fn new_endpoint(&self, tok: mio::Token) -> EndpointFacade {
        EndpointFacade::new(self.id, tok, self.cmd_sender.clone())
    }

    pub fn send(&mut self, buffer: Vec<u8>) -> Result<(), io::Error> {
        self.send_msg(Message::with_body(buffer))
    }

    pub fn send_msg(&mut self, msg: Message) -> Result<(), io::Error> {
        let cmd = SocketCmdSignal::SendMsg(msg);

        try!(self.send_cmd(cmd));

        match self.evt_receiver.recv() {
            Ok(SocketNotify::MsgSent)       => Ok(()),
            Ok(SocketNotify::MsgNotSent(e)) => Err(e),
            Ok(_)                           => Err(other_io_error("unexpected evt")),
            Err(_)                          => Err(other_io_error("evt channel closed"))
        }
    }

    pub fn recv(&mut self) -> Result<Vec<u8>, io::Error> {
        self.recv_msg().map(|msg| msg.to_buffer())
    }

    pub fn recv_msg(&mut self) -> Result<Message, io::Error> {
        let cmd = SocketCmdSignal::RecvMsg;

        try!(self.send_cmd(cmd));

        match self.evt_receiver.recv() {
            Ok(SocketNotify::MsgRecv(msg))  => Ok(msg),
            Ok(SocketNotify::MsgNotRecv(e)) => Err(e),
            Ok(_)                           => Err(other_io_error("unexpected evt")),
            Err(_)                          => Err(other_io_error("evt channel closed"))
        }
    }

    /// Set a socket option.
    /// See [SocketOption](enum.SocketOption.html) to get the list of options.
    pub fn set_option(&mut self, option: SocketOption) -> io::Result<()> {
        let cmd = SocketCmdSignal::SetOption(option);

        try!(self.send_cmd(cmd));

        match self.evt_receiver.recv() {
            Ok(SocketNotify::OptionSet)       => Ok(()),
            Ok(SocketNotify::OptionNotSet(e)) => Err(e),
            Ok(_)                             => Err(other_io_error("unexpected evt")),
            Err(_)                            => Err(other_io_error("evt channel closed"))
        }
    }

    pub fn set_send_timeout(&mut self, timeout: time::Duration) -> io::Result<()> {
        self.set_option(SocketOption::SendTimeout(timeout))
    }

    pub fn set_recv_timeout(&mut self, timeout: time::Duration) -> io::Result<()> {
        self.set_option(SocketOption::RecvTimeout(timeout))
    }

    pub fn set_send_priority(&mut self, priority: u8) -> io::Result<()> {
        self.set_option(SocketOption::SendPriority(priority))
    }

    pub fn set_recv_priority(&mut self, priority: u8) -> io::Result<()> {
        self.set_option(SocketOption::RecvPriority(priority))
    }

    #[doc(hidden)]
    pub fn matches(&self, other: &SocketFacade) -> bool {
        self.socket_type.matches(other.socket_type)
    }

    #[doc(hidden)]
    pub fn forward_msg(&mut self, other: &mut SocketFacade) -> io::Result<()> {
        self.recv_msg().and_then(|msg| other.send_msg(msg))
    }
}

impl Drop for SocketFacade {
    fn drop(&mut self) {
        let cmd = SessionCmdSignal::DestroySocket(self.id);
        let cmd_sig = CmdSignal::Session(cmd);
        let loop_sig = EventLoopSignal::Cmd(cmd_sig);

        let _ = self.cmd_sender.send(loop_sig);
    }
}
