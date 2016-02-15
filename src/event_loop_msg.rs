// Copyright 2016 Benoît Labaere (benoit.labaere@gmail.com)
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0>
// or the MIT license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your option.
// This file may not be copied, modified, or distributed except according to those terms.

use std::sync::mpsc;
use std::io;
use std::time;

use mio;

use global::*;
use Message;

/// Message flowing through the event loop channel so components can communicate with each others.
pub enum EventLoopSignal {
    Cmd(CmdSignal),
    Evt(EvtSignal)
}

impl EventLoopSignal {
    pub fn name(&self) -> &'static str {
        match *self {
            EventLoopSignal::Cmd(_) => "Cmd",
            EventLoopSignal::Evt(_) => "Evt"
        }
    }
}

/// Commands sent by facade components to *backend* components
pub enum CmdSignal {
    Session(SessionCmdSignal),
    Socket(SocketId, SocketCmdSignal)
}

impl CmdSignal {
    pub fn name(&self) -> &'static str {
        match *self {
            CmdSignal::Session(_)  => "Session",
            CmdSignal::Socket(_,_) => "Socket"
        }
    }
}

/// Commands sent to the session
pub enum SessionCmdSignal {
    CreateSocket(SocketType),
    DestroySocket(SocketId),
    CreateProbe(PollRequest),
    Shutdown
}

impl SessionCmdSignal {
    pub fn name(&self) -> &'static str {
        match *self {
            SessionCmdSignal::CreateSocket(_)  => "CreateSocket",
            SessionCmdSignal::DestroySocket(_) => "DestroySocket",
            SessionCmdSignal::CreateProbe(_)   => "CreateProbe",
            SessionCmdSignal::Shutdown         => "Shutdown"
        }
    }
}

/// Commands sent to a socket
pub enum SocketCmdSignal {
    Connect(String),
    Bind(String),
    SendMsg(Message),
    RecvMsg,
    SetOption(SocketOption)
}

impl SocketCmdSignal {
    pub fn name(&self) -> &'static str {
        match *self {
            SocketCmdSignal::Connect(_)     => "Connect",
            SocketCmdSignal::Bind(_)        => "Bind",
            SocketCmdSignal::SendMsg(_)     => "SendMsg",
            SocketCmdSignal::RecvMsg        => "RecvMsg",
            SocketCmdSignal::SetOption(_)   => "SetOption"
        }
    }
}

pub enum SocketOption {
    SendTimeout(time::Duration),
    RecvTimeout(time::Duration),
    Subscribe(String),
    Unsubscribe(String),
    SurveyDeadline(time::Duration),
    ResendInterval(time::Duration)
}

/// Events raised by components living in the event loop, resulting from the execution of commands.
pub enum EvtSignal {
    Socket(SocketId, SocketEvtSignal),
    Pipe(mio::Token, PipeEvtSignal)
}

impl EvtSignal {
    pub fn name(&self) -> &'static str {
        match *self {
            EvtSignal::Socket(_,_) => "Socket",
            EvtSignal::Pipe(_,_)   => "Pipe"
        }
    }
}

// Events raised by sockets
pub enum SocketEvtSignal {
    PipeAdded(mio::Token),
    AcceptorAdded(mio::Token)
}

impl SocketEvtSignal {
    pub fn name(&self) -> &'static str {
        match *self {
            SocketEvtSignal::PipeAdded(_)     => "PipeAdded",
            SocketEvtSignal::AcceptorAdded(_) => "AcceptorAdded"
        }
    }
}

/// Events raised by pipes
pub enum PipeEvtSignal {
    Opened,
    MsgRcv(Message),
    MsgSnd
}

impl PipeEvtSignal {
    pub fn name(&self) -> &'static str {
        match *self {
            PipeEvtSignal::Opened    => "Opened",
            PipeEvtSignal::MsgRcv(_) => "MsgRcv",
            PipeEvtSignal::MsgSnd    => "MsgSnd"
        }
    }
}

/// Events raised by a previoulsy configured timer
pub enum EventLoopTimeout {
    Reconnect(mio::Token, String),
    Rebind(mio::Token, String),
    CancelSend(SocketId),
    CancelRecv(SocketId),
    CancelSurvey(SocketId),
    Resend(SocketId)
}

/// Notifications sent by the *backend* session as reply to the commands sent by the facade session.
pub enum SessionNotify {
    SocketCreated(SocketId, mpsc::Receiver<SocketNotify>),
    ProbeCreated(ProbeId, mpsc::Receiver<PollResult>),
    ProbeNotCreated(io::Error)
}

/// Notifications sent by the *backend* socket as reply to the commands sent by the facade socket.
pub enum SocketNotify {
    Connected,
    NotConnected(io::Error),
    Bound,
    NotBound(io::Error),
    MsgSent,
    MsgNotSent(io::Error),
    MsgRecv(Message),
    MsgNotRecv(io::Error),
    OptionSet,
    OptionNotSet(io::Error)
}

pub struct PollRequest(pub SocketId, pub SocketId);
pub struct PollResult(pub bool, pub bool);
