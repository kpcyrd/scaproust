// Copyright 2016 Benoît Labaere (benoit.labaere@gmail.com)
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0>
// or the MIT license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your option.
// This file may not be copied, modified, or distributed except according to those terms.

use std::rc::Rc;
use std::collections::{ HashMap, HashSet };
use std::sync::mpsc::Sender;
use std::io;

use mio;

use super::{ Protocol, Timeout, clear_timeout };
use pipe::Pipe;
use global::*;
use event_loop_msg::{ SocketNotify };
use EventLoop;
use Message;

pub struct Pub {
    notify_sender: Rc<Sender<SocketNotify>>,
    pipes: HashMap<mio::Token, Pipe>,
    dist: HashSet<mio::Token>
}

impl Pub {
    pub fn new(_: SocketId, notify_tx: Rc<Sender<SocketNotify>>) -> Pub {
        Pub {
            notify_sender: notify_tx,
            pipes: HashMap::new(),
            dist: HashSet::new()
        }
    }

    fn send_notify(&self, evt: SocketNotify) {
        let send_res = self.notify_sender.send(evt);

        if send_res.is_err() {
            error!("Failed to send notify to the facade: '{:?}'", send_res.err());
        }
    }

    fn get_pipe<'a>(&'a mut self, tok: &mio::Token) -> Option<&'a mut Pipe> {
        self.pipes.get_mut(&tok)
    }

    fn broadcast(&mut self, event_loop: &mut EventLoop, msg: Rc<Message>) {
        for tok in self.dist.iter() {
            let msg = msg.clone();

            self.pipes.get_mut(tok).map(|p| p.send_nb(event_loop, msg));
        }
    }
}

impl Protocol for Pub {
    fn get_type(&self) -> SocketType {
        SocketType::Pub
    }

    fn add_pipe(&mut self, tok: mio::Token, pipe: Pipe) -> io::Result<()> {
        match self.pipes.insert(tok, pipe) {
            None    => Ok(()),
            Some(_) => Err(invalid_data_io_error("A pipe has already been added with that token"))
        }
     }

    fn remove_pipe(&mut self, tok: mio::Token) -> Option<Pipe> {
        self.dist.remove(&tok);
        self.pipes.remove(&tok)
    }

    fn open_pipe(&mut self, event_loop: &mut EventLoop, tok: mio::Token) {
        self.pipes.get_mut(&tok).map(|p| p.open(event_loop));
    }

    fn on_pipe_opened(&mut self, event_loop: &mut EventLoop, tok: mio::Token) {
        self.dist.insert(tok);
        self.pipes.get_mut(&tok).map(|p| p.on_open_ack(event_loop));
    }

    fn send(&mut self, event_loop: &mut EventLoop, msg: Message, timeout: Timeout) {
        self.broadcast(event_loop, Rc::new(msg));
        self.send_notify(SocketNotify::MsgSent);
        clear_timeout(event_loop, timeout);
    }

    fn on_send_done(&mut self, _: &mut EventLoop, _: mio::Token) {
    }

    fn on_send_timeout(&mut self, _: &mut EventLoop) {
    }

    fn recv(&mut self, _: &mut EventLoop, _: Timeout) {
        let err = other_io_error("recv not supported by protocol");
        let ntf = SocketNotify::MsgNotRecv(err);

        self.send_notify(ntf);
    }

    fn on_recv_done(&mut self, _: &mut EventLoop, _: mio::Token, _: Message) {
    }

    fn on_recv_timeout(&mut self, _: &mut EventLoop) {
    }

    fn ready(&mut self, event_loop: &mut EventLoop, tok: mio::Token, events: mio::EventSet) {
        self.get_pipe(&tok).map(|p| p.ready(event_loop, events));
    }

    fn destroy(&mut self, event_loop: &mut EventLoop) {
        for (_, pipe) in self.pipes.iter_mut() {
            pipe.close(event_loop);
        }

        self.pipes.clear();
    }
}
