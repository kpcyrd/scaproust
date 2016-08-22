// Copyright 2016 Benoît Labaere (benoit.labaere@gmail.com)
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0>
// or the MIT license <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your option.
// This file may not be copied, modified, or distributed except according to those terms.

use std::ops::Deref;
use std::rc::Rc;

use mio;

use transport::stream::handshake::HandshakeTx; 
use transport::stream::{ 
    StepStream, 
    PipeState,
    transition_if_ok };
use transport::{ Context, PipeEvt };
use Message;

pub struct Initial<T : StepStream + 'static> {
    stream: T,
    proto_ids: (u16, u16)
}

impl<T : StepStream> Initial<T> {
    pub fn new(stream: T, pids: (u16, u16)) -> Initial<T> {
        Initial {
            stream: stream,
            proto_ids: pids
        }
    }
}

impl<T : StepStream> Into<HandshakeTx<T>> for Initial<T> {
    fn into(self) -> HandshakeTx<T> {
        HandshakeTx::new(self.stream, self.proto_ids)
    }
}

impl<T : StepStream> PipeState<T> for Initial<T> {
    fn name(&self) -> &'static str {"Initial"}
    fn open(self: Box<Self>, ctx: &mut Context<PipeEvt>) -> Box<PipeState<T>> {
        let res = ctx.register(self.stream.deref(), mio::EventSet::writable(), mio::PollOpt::level());

        transition_if_ok::<Initial<T>, HandshakeTx<T>, T>(self, ctx, res)
    }
    fn close(self: Box<Self>, ctx: &mut Context<PipeEvt>) -> Box<PipeState<T>> {
        self
    }
    fn send(self: Box<Self>, ctx: &mut Context<PipeEvt>, msg: Rc<Message>) -> Box<PipeState<T>> {
        self
    }
    fn recv(self: Box<Self>, ctx: &mut Context<PipeEvt>) -> Box<PipeState<T>> {
        self
    }
    fn ready(self: Box<Self>, ctx: &mut Context<PipeEvt>, events: mio::EventSet) -> Box<PipeState<T>> {
        self
    }
}

#[cfg(test)]
mod tests {
    use transport::*;
    use transport::tests::*;
    use transport::stream::*;
    use transport::stream::tests::*;
    use transport::stream::initial::*;

    #[test]
    fn on_open_the_stream_should_be_registered() {
        let stream = TestStepStream;
        let state = box Initial::new(stream, (1, 1));
        let mut ctx = TestPipeContext::new();
        let new_state = state.open(&mut ctx);

        assert_eq!(1, ctx.get_registrations().len());
        assert_eq!(0, ctx.get_reregistrations().len());
        assert_eq!(0, ctx.get_deregistrations());

        assert_eq!("HandshakeTx", new_state.name());
    }
}