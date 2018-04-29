extern crate tokio;
extern crate tokio_timer;
extern crate quicr;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate slog;
extern crate slog_term;
extern crate futures;
extern crate url;

use std::net::ToSocketAddrs;
use std::io::{self, Write};

use futures::Future;
use tokio::executor::current_thread::CurrentThread;
use url::Url;

use slog::{Logger, Drain};
use failure::Error;

type Result<T> = std::result::Result<T, Error>;

fn main() {
    let code = {
        let decorator = slog_term::PlainSyncDecorator::new(std::io::stderr());
        let drain = slog_term::FullFormat::new(decorator).use_original_order().build().fuse();
        if let Err(e) = run(Logger::root(drain, o!())) {
            eprintln!("ERROR: {}", e);
            1
        } else { 0 }
    };
    ::std::process::exit(code);
}

fn run(log: Logger) -> Result<()> {
    let url = Url::parse(&::std::env::args().nth(1).ok_or(format_err!("missing address argument"))?)?;
    let remote = url.with_default_port(|_| Ok(4433))?.to_socket_addrs()?.next().ok_or(format_err!("couldn't resolve to an address"))?;

    let mut protocols = Vec::new();
    const PROTO: &[u8] = b"hq-11";
    protocols.push(PROTO.len() as u8);
    protocols.extend_from_slice(PROTO);

    let reactor = tokio::reactor::Reactor::new()?;
    let handle = reactor.handle();
    let timer = tokio_timer::Timer::new(reactor);
    
    let (endpoint, driver, _) = quicr::Endpoint::new()
        .reactor(&handle)
        .timer(timer.handle())
        .logger(log.clone())
        .config(quicr::Config {
            protocols,
            ..quicr::Config::default()
        })
        .bind("[::]:0")?;
    let mut executor = CurrentThread::new_with_park(timer);
    let request = format!("GET {}\r\n", url.path());

    executor.spawn(driver.map_err(|e| eprintln!("IO error: {}", e)));
    executor.block_on(
        endpoint.connect(&remote, url.host_str().map(|x| x.as_bytes()))
            .map_err(|e| format_err!("failed to connect: {}", e))
            .and_then(|(conn, _)| {
                conn.open_bi().map_err(|e| format_err!("failed to open stream: {}", e))
            })
            .and_then(|stream| {
                tokio::io::write_all(stream, request.as_bytes()).map_err(|e| format_err!("failed to send request: {}", e))
            })
            .and_then(|(stream, _)| tokio::io::shutdown(stream).map_err(|e| format_err!("failed to shutdown stream: {}", e)))
            .and_then(|stream| quicr::read_to_end(stream, usize::max_value()).map_err(|e| format_err!("failed to read response: {}", e)))
            .map(|(_, data)| {
                io::stdout().write_all(&data).unwrap();
                io::stdout().flush().unwrap();
            })
    ).map_err(|e| e.into_inner().unwrap())?;

    Ok(())
}