mod impls;
mod wrapper_fns;
pub(crate) use wrapper_fns::*;

use super::{
    super::stream::{pipe_mode, PipeModeTag, REUNITE_ERROR_MSG},
    imports::*,
};
use std::{
    error::Error,
    fmt::{self, Display, Formatter},
    io,
    marker::PhantomData,
    sync::Arc,
};

/// A Tokio-based named pipe stream, created by a server-side listener or by connecting to a server.
///
/// This type combines in itself all possible combinations of receive modes and send modes, plugged into it using the `Rm` and `Sm` generic parameters respectively.
///
/// Pipe streams can be split by reference and by value for concurrent receive and send operations. Splitting by reference is ephemeral and can be achieved by simply borrowing the stream, since both `PipeStream` and `&PipeStream` implement I/O traits. Splitting by value is done using the [`.split()`](Self::split) method, producing a [`RecvHalf`] and a [`SendHalf`], and can be reverted via the `.reunite()` method defined on the halves.
///
/// # Examples
///
/// ## Basic client
/// ```no_run
/// use futures::{prelude::*, try_join};
/// use interprocess::os::windows::named_pipe::{pipe_mode, tokio::*};
///
/// // Await this here since we can't do a whole lot without a connection.
/// let conn = DuplexPipeStream::<pipe_mode::Bytes>::connect("Example").await?;
///
/// // This consumes our connection and splits it into two owned halves, so that we could
/// // concurrently act on both. Take care not to use the .split() method from the futures crate's
/// // AsyncReadExt.
/// let (mut reader, mut writer) = conn.split();
///
/// // Preemptively allocate a sizeable buffer for reading.
/// // This size should be enough and should be easy to find for the allocator.
/// let mut buffer = String::with_capacity(128);
///
/// // Describe the write operation as writing our whole string, waiting for
/// // that to complete, and then shutting down the write half, which sends
/// // an EOF to the other end to help it determine where the message ends.
/// let write = async {
///     writer.write_all(b"Hello from client!").await?;
///     // Because only the trait from futures is implemented for now, it's "close" instead of
///     // "shutdown".
///     writer.close().await?;
///     Ok(())
/// };
///
/// // Describe the read operation as reading until EOF into our big buffer.
/// let read = reader.read_to_string(&mut buffer);
///
/// // Concurrently perform both operations: write-and-send-EOF and read.
/// try_join!(write, read)?;
///
/// // Get rid of those here to close the read half too.
/// drop((reader, writer));
///
/// // Display the results when we're done!
/// println!("Server answered: {}", buffer.trim());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct PipeStream<Rm: PipeModeTag, Sm: PipeModeTag> {
    raw: RawPipeStream,
    flush: TokioMutex<Option<FlushJH>>,
    _phantom: PhantomData<(Rm, Sm)>,
}
type FlushJH = TokioJoinHandle<io::Result<()>>;

/// Type alias for a Tokio-based pipe stream with the same read mode and write mode.
pub type DuplexPipeStream<M> = PipeStream<M, M>;

/// Type alias for a pipe stream with a read mode but no write mode.
pub type RecvPipeStream<M> = PipeStream<M, pipe_mode::None>;
/// Type alias for a pipe stream with a write mode but no read mode.
pub type SendPipeStream<M> = PipeStream<pipe_mode::None, M>;

/// The receiving half of a [`PipeStream`] as produced via `.split()`.
pub struct RecvHalf<Rm: PipeModeTag> {
    raw: Arc<RawPipeStream>,
    _phantom: PhantomData<Rm>,
}

/// The sending half of a [`PipeStream`] as produced via `.split()`.
pub struct SendHalf<Sm: PipeModeTag> {
    raw: Arc<RawPipeStream>,
    flush: TokioMutex<Option<FlushJH>>,
    _phantom: PhantomData<Sm>,
}

pub(crate) enum RawPipeStream {
    Server(TokioNPServer),
    Client(TokioNPClient),
}

/// Additional contextual information for conversions from a raw handle to a named pipe stream.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum FromRawHandleErrorKind {
    /// It wasn't possible to determine whether the pipe handle corresponds to a pipe server or a pipe client.
    IsServerCheckFailed,
    /// The type being converted into has message semantics, but it wasn't possible to determine whether message boundaries are preserved in the pipe.
    MessageBoundariesCheckFailed,
    /// The type being converted into has message semantics, but message boundaries are not preserved in the pipe.
    NoMessageBoundaries,
    /// An error was reported by Tokio.
    ///
    /// Most of the time, this means that `from_raw_handle()` call was performed outside of the Tokio runtime, but OS errors associated with the registration of the handle in the runtime belong to this category as well.
    TokioError,
}
/// Error type for `from_raw_handle()` constructors.
pub type FromRawHandleError = (FromRawHandleErrorKind, io::Error);

/// Error type for `.reunite()` on split receive and send halves.
///
/// The error indicates that the halves belong to different streams and allows to recover both of them.
#[derive(Debug)]
pub struct ReuniteError<Rm: PipeModeTag, Sm: PipeModeTag> {
    /// The receive half that didn't go anywhere, in case you still need it.
    pub recv_half: RecvHalf<Rm>,
    /// The send half that didn't go anywhere, in case you still need it.
    pub send_half: SendHalf<Sm>,
}
impl<Rm: PipeModeTag, Sm: PipeModeTag> Display for ReuniteError<Rm, Sm> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.pad(REUNITE_ERROR_MSG)
    }
}
impl<Rm: PipeModeTag, Sm: PipeModeTag> Error for ReuniteError<Rm, Sm> {}
