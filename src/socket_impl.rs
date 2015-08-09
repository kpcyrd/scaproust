use std::rc::Rc;
use std::collections::hash_map::*;
use std::sync::mpsc::Sender;
use std::io;

use mio;

use global::*;
use event_loop_msg::*;

use protocol::Protocol as Protocol;
use pipe::Pipe;
use acceptor::Acceptor;
use transport::{create_transport, Connection, Listener};

use EventLoop;
use Message;

pub struct SocketImpl {
	id: SocketId,
	protocol: Box<Protocol>,
	evt_sender: Rc<Sender<SocketEvt>>,
	acceptors: HashMap<mio::Token, Acceptor>,
	id_seq: IdSequence,
	added_tokens: Option<Vec<mio::Token>>,
	removed_tokens: Option<Vec<mio::Token>>
}

impl SocketImpl {

	pub fn new(id: SocketId, proto: Box<Protocol>, evt_tx: Rc<Sender<SocketEvt>>, id_seq: IdSequence) -> SocketImpl {
		SocketImpl { 
			id: id,
			protocol: proto, 
			evt_sender: evt_tx,
			acceptors: HashMap::new(),
			id_seq: id_seq,
			added_tokens: None,
			removed_tokens: None
		}
	}

	fn send_evt(&self, evt: SocketEvt) {
		let send_res = self.evt_sender.send(evt);

		if send_res.is_err() {
			error!("[{:?}] failed to notify event to session: '{:?}'", self.id, send_res.err());
		} 
	}

	pub fn connect(&mut self, addr: String, event_loop: &mut EventLoop, token: mio::Token) {
		debug!("[{:?}] pipe [{:?}] connect: '{}'", self.id, token, addr);

		let connect_result = self.
			create_connection(&addr).
			and_then(|conn| self.on_connected(Some(addr), event_loop, token, conn));
		let evt = match connect_result {
			Ok(_) => SocketEvt::Connected,
			Err(e) => SocketEvt::NotConnected(e)
		};

		self.send_evt(evt);
	}

	pub fn reconnect(&mut self, addr: String, event_loop: &mut EventLoop, token: mio::Token) {
		debug!("[{:?}] pipe [{:?}] reconnect: '{}'", self.id, token, addr);

		self.create_connection(&addr).
			and_then(|c| self.on_connected(Some(addr), event_loop, token, c)).
			unwrap_or_else(|e| self.on_pipe_error(event_loop, token, e));
	}

	fn on_pipe_error(&mut self, event_loop: &mut EventLoop, token: mio::Token, err: io::Error) {
		debug!("[{:?}] pipe [{:?}] error: '{:?}'", self.id, token, err);

		if let Some(pipe) = self.protocol.remove_pipe(token) {
			let _ = pipe.close(event_loop);
			if let Some(addr) = pipe.addr() {
				let _ = event_loop.
					timeout_ms(EventLoopTimeout::Reconnect(token, addr), 200).
					map_err(|err| error!("[{:?}] pipe [{:?}] reconnect timeout failed: '{:?}'", self.id, token, err));
			}
		}
	}

	fn create_connection(&self, addr: &str) -> io::Result<Box<Connection>> {

		let addr_parts: Vec<&str> = addr.split("://").collect();
		let scheme = addr_parts[0];
		let specific_addr = addr_parts[1];
		let transport = create_transport(scheme);
		
		transport.connect(specific_addr)
	}

	fn on_connected(&mut self, addr: Option<String>, event_loop: &mut EventLoop, token: mio::Token, conn: Box<Connection>) -> io::Result<()> {
		let protocol_ids = (self.protocol.id(), self.protocol.peer_id());
		let pipe = Pipe::new(token, addr, protocol_ids, conn);

		pipe.open(event_loop).and_then(|_| Ok(self.protocol.add_pipe(token, pipe)))
	}

	pub fn bind(&mut self, addr: String, event_loop: &mut EventLoop, token: mio::Token) {
		debug!("[{:?}] acceptor [{:?}] bind: '{}'", self.id, token, addr);

		let evt = match self.create_listener(&addr).and_then(|c| self.on_listener_created(addr, event_loop, token, c)) {
			Ok(_) => SocketEvt::Bound,
			Err(e) => SocketEvt::NotBound(e)
		};

		self.send_evt(evt);
	}

	pub fn rebind(&mut self, addr: String, event_loop: &mut EventLoop, token: mio::Token) {
		debug!("[{:?}] acceptor [{:?}] rebind: '{}'", self.id, token, addr);

		self.create_listener(&addr).
			and_then(|c| self.on_listener_created(addr, event_loop, token, c)).
			unwrap_or_else(|e| self.on_acceptor_error(event_loop, token, e));
	}

	fn create_listener(&self, addr: &str) -> io::Result<Box<Listener>> {

		let addr_parts: Vec<&str> = addr.split("://").collect();
		let scheme = addr_parts[0];
		let specific_addr = addr_parts[1];
		let transport = create_transport(scheme);
		
		transport.bind(specific_addr)
	}

	fn on_listener_created(&mut self, addr: String, event_loop: &mut EventLoop, id: mio::Token, listener: Box<Listener>) -> io::Result<()> {
		let mut acceptor = Acceptor::new(id, addr, listener);

		acceptor.open(event_loop).and_then(|_| Ok(self.add_acceptor(id, acceptor)))
	}

	fn add_acceptor(&mut self, token: mio::Token, acceptor: Acceptor) {
		self.acceptors.insert(token, acceptor);
	}

	fn remove_acceptor(&mut self, token: mio::Token) -> Option<Acceptor> {
		self.acceptors.remove(&token)
	}

	pub fn ready(&mut self, event_loop: &mut EventLoop, token: mio::Token, events: mio::EventSet) -> Option<Vec<mio::Token>> {

		if self.acceptors.contains_key(&token) {
			self.acceptor_ready(event_loop, token, events)
		} else {
			self.pipe_ready(event_loop, token, events)
		}

		self.added_tokens.take()
	}

	fn acceptor_ready(&mut self, event_loop: &mut EventLoop, token: mio::Token, events: mio::EventSet) {
		debug!("[{:?}] acceptor [{:?}] ready: '{:?}'", self.id, token, events);

		self.acceptors.get_mut(&token).unwrap().
			ready(event_loop, events).
			and_then(|conns| self.on_connections_accepted(event_loop, conns)).
			unwrap_or_else(|e| self.on_acceptor_error(event_loop, token, e));
	}

	fn pipe_ready(&mut self, event_loop: &mut EventLoop, token: mio::Token, events: mio::EventSet) {
		debug!("[{:?}] pipe [{:?}] ready: '{:?}'", self.id, token, events);

		self.protocol.
			ready(event_loop, token, events).
			unwrap_or_else(|e| self.on_pipe_error(event_loop, token, e));
	}

	fn on_connections_accepted(&mut self, event_loop: &mut EventLoop, mut conns: Vec<Box<Connection>>) -> io::Result<()> {
		let tokens: Vec<mio::Token> = conns.drain(..).
			map(|conn| self.on_connection_accepted(event_loop, conn)).
			collect();

		self.added_tokens = Some(tokens);

		Ok(())
	}

	fn on_connection_accepted(&mut self, event_loop: &mut EventLoop, conn: Box<Connection>) -> mio::Token {
		let token = mio::Token(self.id_seq.next());

		debug!("[{:?}] on connection accepted: '{:?}'", self.id, token);

		self.
			on_connected(None, event_loop, token, conn).
			unwrap_or_else(|e| self.on_pipe_error(event_loop, token, e));

		token
	}

	fn on_acceptor_error(&mut self, event_loop: &mut EventLoop, token: mio::Token, err: io::Error) {
		debug!("[{:?}] acceptor [{:?}] error: '{:?}'", self.id, token, err);

		if let Some(mut acceptor) = self.remove_acceptor(token) {
			acceptor.
				close(event_loop).
				unwrap_or_else(|err| debug!("[{:?}] acceptor [{:?}] error while closing: '{:?}'", self.id, token, err));
			let _ = event_loop.
				timeout_ms(EventLoopTimeout::Rebind(token, acceptor.addr()), 200).
				map_err(|err| error!("[{:?}] acceptor [{:?}] reconnect timeout failed: '{:?}'", self.id, token, err));

		}
	}

	pub fn send(&mut self, event_loop: &mut EventLoop, msg: Message) {
		debug!("[{:?}] send", self.id);

		let _ = event_loop.timeout_ms(EventLoopTimeout::CancelSend(self.id), 1000).
			map(|timeout| self.protocol.send(event_loop, msg, Box::new(move |el: &mut EventLoop| {el.clear_timeout(timeout)}))).
			map_err(|err| error!("[{:?}] failed to set timeout on send: '{:?}'", self.id, err));
	}

	pub fn on_send_timeout(&mut self, event_loop: &mut EventLoop) {
		debug!("[{:?}] on_send_timeout", self.id);
		self.protocol.on_send_timeout(event_loop);
	}

	pub fn recv(&mut self, event_loop: &mut EventLoop) {
		debug!("[{:?}] recv", self.id);

		let _ = event_loop.timeout_ms(EventLoopTimeout::CancelRecv(self.id), 1000).
			map(|timeout| self.protocol.recv(event_loop, Box::new(move |el: &mut EventLoop| {el.clear_timeout(timeout)}))).
			map_err(|err| error!("[{:?}] failed to set timeout on recv: '{:?}'", self.id, err));
	}

	pub fn on_recv_timeout(&mut self, event_loop: &mut EventLoop) {
		debug!("[{:?}] on_recv_timeout", self.id);
		self.protocol.on_recv_timeout(event_loop);
	}
}
