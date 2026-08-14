use super::*;

struct PortableOutputStream {
    name: &'static str,
    file: File,
    offset: u64,
    dropped: u64,
    writer_cursor: File,
    reader_cursor: File,
    partial: Vec<u8>,
    discarding_oversized: bool,
}

pub(super) struct PortableOutputReaders {
    streams: Vec<PortableOutputStream>,
    pending: Vec<OutputEvent>,
    last_flush: Instant,
    reported_events: usize,
    coalesced_reported: bool,
    cursor_dirty: bool,
}

impl PortableOutputReaders {
    pub(super) fn open(paths: &OutputSinkPaths) -> Result<Self, String> {
        let streams = [
            (
                "stdout",
                &paths.stdout,
                &paths.stdout_writer_cursor,
                &paths.stdout_reader_cursor,
            ),
            (
                "stderr",
                &paths.stderr,
                &paths.stderr_writer_cursor,
                &paths.stderr_reader_cursor,
            ),
        ]
        .into_iter()
        .map(|(name, sink, writer_path, reader_path)| {
            let writer_cursor = initialize_cursor(writer_path)?;
            let reader_cursor = initialize_cursor(reader_path)?;
            let file = OpenOptions::new()
                .read(true)
                .open(sink)
                .map_err(|error| format!("open portable executor {name}: {error}"))?;
            Ok(PortableOutputStream {
                name,
                file,
                offset: 0,
                dropped: 0,
                writer_cursor,
                reader_cursor,
                partial: Vec::new(),
                discarding_oversized: false,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            streams,
            pending: Vec::new(),
            last_flush: Instant::now(),
            reported_events: 0,
            coalesced_reported: false,
            cursor_dirty: false,
        })
    }

    pub(super) fn poll(&mut self) -> Result<usize, String> {
        let mut consumed_total = 0_usize;
        for stream in &mut self.streams {
            let total = stream
                .file
                .metadata()
                .map_err(|error| format!("inspect portable executor {}: {error}", stream.name))?
                .len();
            if total < stream.offset {
                return Err(format!(
                    "portable executor {} output regressed from {} to {total}",
                    stream.name, stream.offset
                ));
            }
            let available = total - stream.offset;
            if available > OUTPUT_SINK_LIMIT {
                let dropped = available - OUTPUT_SINK_LIMIT;
                stream.offset += dropped;
                stream.dropped = stream.dropped.saturating_add(dropped);
                stream.partial.clear();
                stream.discarding_oversized = false;
                self.pending.push(OutputEvent {
                    stream: stream.name,
                    line: format!("executor output dropped {dropped} bytes before framing"),
                    truncated: true,
                    io_error: false,
                    dropped,
                });
            }
            let writer = read_output_cursor(&stream.writer_cursor)?;
            if writer.total != total || writer.dropped != stream.dropped {
                write_output_cursor(
                    &stream.writer_cursor,
                    OutputCursor {
                        generation: writer.generation,
                        total,
                        dropped: stream.dropped,
                    },
                )?;
            }
            let mut remaining = OUTPUT_READ_LIMIT;
            while remaining > 0
                && stream.offset < total
                && self.pending.len() < OUTPUT_EVENTS_PER_HEARTBEAT
            {
                let mut buffer = [0_u8; 4_096];
                let requested = remaining
                    .min(buffer.len())
                    .min((total - stream.offset) as usize);
                let count = read_at_portable(&stream.file, &mut buffer[..requested], stream.offset)
                    .map_err(|error| {
                        format!("read portable executor {} output: {error}", stream.name)
                    })?;
                if count == 0 {
                    return Err(format!(
                        "portable executor {} output ended before observed length",
                        stream.name
                    ));
                }
                let consumed = frame_output(stream, &buffer[..count], &mut self.pending);
                stream.offset += consumed as u64;
                remaining -= consumed;
                consumed_total += consumed;
                self.cursor_dirty = true;
                if consumed < count {
                    break;
                }
            }
        }
        Ok(consumed_total)
    }

    pub(super) fn flush_if_due(
        &mut self,
        state_path: &Path,
        event_log: &Path,
        state: &mut PersistedInvocation,
        force: bool,
    ) -> Result<bool, String> {
        if force {
            for stream in &mut self.streams {
                if !stream.partial.is_empty() {
                    self.pending.push(OutputEvent {
                        stream: stream.name,
                        line: String::from_utf8_lossy(&stream.partial).to_string(),
                        truncated: stream.discarding_oversized,
                        io_error: false,
                        dropped: 0,
                    });
                    stream.partial.clear();
                }
            }
        }
        if self.pending.is_empty() && !(force && self.cursor_dirty) {
            return Ok(false);
        }
        if !force && self.last_flush.elapsed() < OUTPUT_HEARTBEAT_INTERVAL {
            return Ok(false);
        }
        let mut events = std::mem::take(&mut self.pending);
        let remaining = OUTPUT_EVENTS_PER_INVOCATION.saturating_sub(self.reported_events);
        let suppressed_bytes = events
            .iter()
            .skip(remaining)
            .map(|event| event.line.len() as u64 + 1)
            .sum::<u64>();
        events.truncate(remaining);
        self.reported_events += events.len();
        if suppressed_bytes > 0 && !self.coalesced_reported {
            events.push(OutputEvent {
                stream: "combined",
                line: "executor output was coalesced after the bounded progress sample".to_string(),
                truncated: true,
                io_error: false,
                dropped: suppressed_bytes,
            });
            self.coalesced_reported = true;
        }
        if !events.is_empty() {
            state.progress_at = unix_now()?;
            write_invocation_atomic(state_path, state)?;
            for event in &events {
                append_executor_event(
                    event_log,
                    state,
                    if event.dropped > 0 {
                        "child_output_dropped"
                    } else {
                        "child_output"
                    },
                    Some(serde_json::json!({
                        "stream": event.stream,
                        "output": event.line,
                        "truncated": event.truncated,
                        "dropped_bytes": event.dropped,
                    })),
                )?;
            }
        }
        self.persist_reader_cursors()?;
        self.last_flush = Instant::now();
        Ok(!events.is_empty())
    }

    fn persist_reader_cursors(&mut self) -> Result<(), String> {
        for stream in &mut self.streams {
            let current = read_output_cursor(&stream.reader_cursor)?;
            write_output_cursor(
                &stream.reader_cursor,
                OutputCursor {
                    generation: current.generation,
                    total: stream.offset.saturating_sub(stream.partial.len() as u64),
                    dropped: stream.dropped,
                },
            )?;
        }
        self.cursor_dirty = false;
        Ok(())
    }

    pub(super) fn drain_after_exit(
        &mut self,
        state_path: &Path,
        event_log: &Path,
        state: &mut PersistedInvocation,
        renewal: &mut ClaimRenewalSchedule,
    ) -> Result<CompletionDrainOutcome, String> {
        if let Some(outcome) = renew_claim_if_due(renewal, state)? {
            return Ok(outcome);
        }
        #[cfg(test)]
        if let Ok(marker) = std::env::var("AUTOSPEC_TEST_COMPLETION_DRAIN_MARKER") {
            fs::write(marker, b"entered\n")
                .map_err(|error| format!("write test completion drain marker: {error}"))?;
        }
        #[cfg(test)]
        if let Ok(delay) = std::env::var("AUTOSPEC_TEST_COMPLETION_DRAIN_DELAY_MS") {
            let delay = delay
                .parse::<u64>()
                .map_err(|_| "invalid test completion drain delay".to_string())?;
            thread::sleep(Duration::from_millis(delay));
        }
        let byte_budget = OUTPUT_SINK_LIMIT
            .checked_mul(self.streams.len() as u64)
            .ok_or_else(|| "portable executor output drain budget overflowed".to_string())?;
        let mut consumed_total = 0_u64;
        loop {
            if let Some(outcome) = renew_claim_if_due(renewal, state)? {
                return Ok(outcome);
            }
            let consumed = self.poll()? as u64;
            consumed_total = consumed_total.saturating_add(consumed);
            if consumed_total > byte_budget {
                return Err("portable executor output drain exceeded its fixed bound".to_string());
            }
            if consumed == 0 {
                self.flush_if_due(state_path, event_log, state, true)?;
                return Ok(CompletionDrainOutcome::Drained);
            }
            self.last_flush = Instant::now() - OUTPUT_HEARTBEAT_INTERVAL;
            self.flush_if_due(state_path, event_log, state, false)?;
        }
    }
}

fn renew_claim_if_due(
    renewal: &mut ClaimRenewalSchedule,
    state: &PersistedInvocation,
) -> Result<Option<CompletionDrainOutcome>, String> {
    if !renewal.is_due() {
        return Ok(None);
    }
    match refresh_bridge_claim(state, ClaimRenewalPhase::Implementation) {
        Ok(BridgeClaimOwnership::Refreshed { ttl_seconds }) => {
            renewal.mark_refreshed(ttl_seconds);
            Ok(None)
        }
        Ok(BridgeClaimOwnership::Lost) => Ok(Some(CompletionDrainOutcome::OwnershipLost)),
        Err(error) => Ok(Some(CompletionDrainOutcome::TransientFailure(error))),
    }
}

fn initialize_cursor(path: &Path) -> Result<File, String> {
    let cursor = open_private_file(path, true)?;
    cursor
        .set_len(OUTPUT_CURSOR_FILE_BYTES)
        .map_err(|error| format!("size portable executor output cursor: {error}"))?;
    write_output_cursor(&cursor, OutputCursor::default())?;
    Ok(cursor)
}

fn frame_output(
    stream: &mut PortableOutputStream,
    bytes: &[u8],
    events: &mut Vec<OutputEvent>,
) -> usize {
    for (index, byte) in bytes.iter().enumerate() {
        if stream.discarding_oversized {
            if *byte == b'\n' {
                stream.discarding_oversized = false;
            }
            continue;
        }
        if *byte == b'\n' {
            events.push(OutputEvent {
                stream: stream.name,
                line: String::from_utf8_lossy(&stream.partial).to_string(),
                truncated: false,
                io_error: false,
                dropped: 0,
            });
            stream.partial.clear();
        } else if stream.partial.len() < OUTPUT_LINE_LIMIT {
            stream.partial.push(*byte);
        } else {
            events.push(OutputEvent {
                stream: stream.name,
                line: String::from_utf8_lossy(&stream.partial).to_string(),
                truncated: true,
                io_error: false,
                dropped: 0,
            });
            stream.partial.clear();
            stream.discarding_oversized = true;
        }
        if events.len() >= OUTPUT_EVENTS_PER_HEARTBEAT {
            return index + 1;
        }
    }
    bytes.len()
}
