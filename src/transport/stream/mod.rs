// Copyright 2016 Benoît Labaere (benoit.labaere@gmail.com)
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0>
// or the MIT license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your option.
// This file may not be copied, modified, or distributed except according to those terms.

/// This module defines various building blocks for transport that uses mio streams.

mod initial;
mod handshake;
mod dead;

use std::ops::Deref;
use std::rc::Rc;
use std::io;

use byteorder::{ BigEndian, ByteOrder };

use mio;

use super::*;
use io_error::*;
use Message;

pub trait Sender {
    fn start_send(&mut self, msg: Rc<Message>) -> io::Result<bool>;
    fn resume_send(&mut self) -> io::Result<bool>;
    fn has_pending_send(&self) -> bool;
}

pub trait Receiver {
    fn start_recv(&mut self) -> io::Result<Option<Message>>;
    fn resume_recv(&mut self) -> io::Result<Option<Message>>;
    fn has_pending_recv(&self) -> bool;
}

pub trait Handshake {
    fn send_handshake(&mut self, pids: (u16, u16)) -> io::Result<()>;
    fn recv_handshake(&mut self, pids: (u16, u16)) -> io::Result<()>;
}

pub trait StepStream : Sender + Receiver + Handshake + Deref<Target=mio::Evented> {
}

pub trait PipeState<T : StepStream> {
    fn name(&self) -> &'static str;
    fn open(self: Box<Self>, ctx: &mut Context<PipeEvt>) -> Box<PipeState<T>>;
    fn close(self: Box<Self>, ctx: &mut Context<PipeEvt>) -> Box<PipeState<T>>;
    fn send(self: Box<Self>, ctx: &mut Context<PipeEvt>, msg: Rc<Message>) -> Box<PipeState<T>>;
    fn recv(self: Box<Self>, ctx: &mut Context<PipeEvt>) -> Box<PipeState<T>>;

    fn error(self: Box<Self>, ctx: &mut Context<PipeEvt>, err: io::Error) -> Box<PipeState<T>> {
        box dead::Dead
    }

    fn ready(self: Box<Self>, ctx: &mut Context<PipeEvt>, events: mio::EventSet) -> Box<PipeState<T>>;
}

pub struct Pipe<T : StepStream + 'static> {
    state: Option<Box<PipeState<T>>>
}

impl<T : StepStream + 'static> Pipe<T> {
    pub fn new(stream: T, pids: (u16, u16)) -> Pipe<T> {
        let initial_state = box initial::Initial::new(stream, pids);

        Pipe { state: Some(initial_state) }
    }

    fn apply<F>(&mut self, transition: F) where F : FnOnce(Box<PipeState<T>>) -> Box<PipeState<T>> {
        if let Some(old_state) = self.state.take() {
            let new_state = transition(old_state);

            self.state = Some(new_state);
        }
    }
}

impl<T : StepStream> Endpoint<PipeCmd, PipeEvt> for Pipe<T> {
    fn ready(&mut self, ctx: &mut Context<PipeEvt>, events: mio::EventSet) {
        self.apply(|s| s.ready(ctx, events))
    }
    fn process(&mut self, ctx: &mut Context<PipeEvt>, cmd: PipeCmd) {
        match cmd {
            PipeCmd::Open      => self.apply(|s| s.open(ctx)),
            PipeCmd::Close     => self.apply(|s| s.close(ctx)),
            PipeCmd::Send(msg) => self.apply(|s| s.send(ctx, msg)),
            PipeCmd::Recv      => self.apply(|s| s.recv(ctx))
        }
    }
}

pub trait WriteBuffer {
    fn write_buffer(&mut self, buffer: &[u8], written: &mut usize) -> io::Result<bool>;
}

impl<T:io::Write> WriteBuffer for T {
    fn write_buffer(&mut self, buf: &[u8], written: &mut usize) -> io::Result<bool> {
        *written += try!(self.write(&buf[*written..]));

        Ok(*written == buf.len())
    }
}

pub fn send_and_check_handshake<T:io::Write>(stream: &mut T, pids: (u16, u16)) -> io::Result<()> {
    let (proto_id, _) = pids;
    let handshake = create_handshake(proto_id);

    match try!(stream.write(&handshake)) {
        8 => Ok(()),
        _ => Err(would_block_io_error("failed to send handshake"))
    }
}

fn create_handshake(protocol_id: u16) -> [u8; 8] {
    // handshake is Zero, 'S', 'P', Version, Proto[2], Rsvd[2]
    let mut handshake = [0, 83, 80, 0, 0, 0, 0, 0];
    BigEndian::write_u16(&mut handshake[4..6], protocol_id);
    handshake
}

pub fn recv_and_check_handshake<T:io::Read>(stream: &mut T, pids: (u16, u16)) -> io::Result<()> {
    let mut handshake = [0u8; 8];

    stream.read(&mut handshake).and_then(|_| check_handshake(pids, &handshake))
}

fn check_handshake(pids: (u16, u16), handshake: &[u8; 8]) -> io::Result<()> {
    let (_, proto_id) = pids;
    let expected_handshake = create_handshake(proto_id);

    if handshake == &expected_handshake {
        Ok(())
    } else {
        Err(invalid_data_io_error("received bad handshake"))
    }
}

pub fn transition<F, T, S>(f: Box<F>) -> Box<T> where
    F : PipeState<S>,
    F : Into<T>,
    T : PipeState<S>,
    S : StepStream
{
    box Into::into(*f)
}
fn transition_if_ok<F, T : 'static, S>(f: Box<F>, ctx: &mut Context<PipeEvt>, res: io::Result<()>) -> Box<PipeState<S>> where
    F : PipeState<S>,
    F : Into<T>,
    T : PipeState<S>,
    S : StepStream
{
    match res {
        Ok(..) => transition::<F, T, S>(f),
        Err(e) => f.error(ctx, e)
    }
}
fn no_transition_if_ok<F : 'static, S>(f: Box<F>, ctx: &mut Context<PipeEvt>, res: io::Result<()>) -> Box<PipeState<S>> where
    F : PipeState<S>,
    S : StepStream
{
    match res {
        Ok(..) => f,
        Err(e) => f.error(ctx, e)
    }
}


#[cfg(test)]
mod tests {
    use std::ops::Deref;
    use std::rc::Rc;
    use std::io;

    use mio;

    use transport::stream;
    use Message;

    pub struct TestStepStream;

    impl stream::StepStream for TestStepStream {
    }

    impl mio::Evented for TestStepStream {
        fn register(&self, poll: &mio::Poll, token: mio::Token, interest: mio::EventSet, opts: mio::PollOpt) -> io::Result<()> {
            unimplemented!();
        }
        fn reregister(&self, poll: &mio::Poll, token: mio::Token, interest: mio::EventSet, opts: mio::PollOpt) -> io::Result<()> {
            unimplemented!();
        }
        fn deregister(&self, poll: &mio::Poll) -> io::Result<()> {
            unimplemented!();
        }
    }

    impl Deref for TestStepStream {
        type Target = mio::Evented;
        fn deref(&self) -> &Self::Target {
            self
        }
    }

    impl stream::Sender for TestStepStream {
        fn start_send(&mut self, msg: Rc<Message>) -> io::Result<bool> {
            unimplemented!();
        }

        fn resume_send(&mut self) -> io::Result<bool> {
            unimplemented!();
        }

        fn has_pending_send(&self) -> bool {
            unimplemented!();
        }
    }


    impl stream::Receiver for TestStepStream {
        fn start_recv(&mut self) -> io::Result<Option<Message>> {
            unimplemented!();
        }

        fn resume_recv(&mut self) -> io::Result<Option<Message>> {
            unimplemented!();
        }

        fn has_pending_recv(&self) -> bool {
            unimplemented!();
        }
    }

    impl stream::Handshake for TestStepStream {
        fn send_handshake(&mut self, pids: (u16, u16)) -> io::Result<()> {
            unimplemented!();
        }
        fn recv_handshake(&mut self, pids: (u16, u16)) -> io::Result<()> {
            unimplemented!();
        }
    }

}