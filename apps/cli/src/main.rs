//! Receive-only local CLI. No command exposes output, rig, PTT, or transmit.

fn main() -> anyhow::Result<()> {
    use anyhow::{Context, bail};
    use slotpilot_api::{
        Command, InputConfiguration, InputDeviceIdentity, ReceiveSelection, SubscriptionRequest,
    };
    use slotpilot_domain::RequestId;
    use slotpilot_ipc::{CancellationToken, EndpointAddress};

    let mut arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let json = remove_flag(&mut arguments, "--json");
    let jsonl = remove_flag(&mut arguments, "--jsonl");
    let _non_interactive = remove_flag(&mut arguments, "--non-interactive");
    let request_id = remove_option(&mut arguments, "--request-id")
        .map(|value| value.parse::<RequestId>())
        .transpose()
        .context("invalid --request-id")?;

    let (runtime_directory, command) = match arguments.as_slice() {
        [mode, runtime] if mode == "phase0-status" || mode == "status" => {
            (runtime.clone(), Command::GetSnapshot)
        }
        [devices, audio, list, runtime]
            if devices == "devices" && audio == "audio" && list == "list" =>
        {
            (runtime.clone(), Command::ListInputDevices)
        }
        [receive, status, runtime] if receive == "receive" && status == "status" => {
            (runtime.clone(), Command::GetReceiveStatus)
        }
        [receive, stop, runtime] if receive == "receive" && stop == "stop" => {
            (runtime.clone(), Command::ReceiveStop)
        }
        [receive, history, runtime, after, limit]
            if receive == "receive" && history == "history" =>
        {
            (
                runtime.clone(),
                Command::QueryReceiveHistory {
                    after_sequence: after.parse().context("invalid history cursor")?,
                    limit: limit.parse().context("invalid history limit")?,
                },
            )
        }
        [
            receive,
            start,
            runtime,
            platform,
            opaque_id,
            rate,
            channels,
            format,
            channel,
        ] if receive == "receive" && start == "start" => (
            runtime.clone(),
            Command::ReceiveStart {
                selection: ReceiveSelection {
                    device_identity: InputDeviceIdentity {
                        platform: parse_platform(platform)?,
                        opaque_id: opaque_id.clone(),
                    },
                    configuration: InputConfiguration {
                        sample_rate_hz: rate.parse().context("invalid sample rate")?,
                        channels: channels.parse().context("invalid channel count")?,
                        sample_format: parse_sample_format(format)?,
                        selected_channel: channel.parse().context("invalid selected channel")?,
                    },
                },
            },
        ),
        [events, follow, runtime, after, limit] if events == "events" && follow == "follow" => {
            let address = EndpointAddress::for_user(runtime, "phase0-dev")?;
            let snapshot = slotpilot::request_snapshot(
                &address,
                "req_eventssnapshot"
                    .parse()
                    .context("invalid built-in snapshot request identity")?,
                &CancellationToken::new(),
            )?;
            let slotpilot_api::ResponseOutcome::Success(slotpilot_api::ResultBody::Snapshot(
                snapshot,
            )) = snapshot.outcome
            else {
                bail!("could not obtain a coherent event snapshot");
            };
            let response = slotpilot::request_events(
                &address,
                &SubscriptionRequest {
                    api_version: slotpilot_api::API_VERSION,
                    after: Some(slotpilot_api::EventCursor {
                        service_instance_id: snapshot.service_instance_id,
                        sequence: after.parse().context("invalid event cursor")?,
                    }),
                    limit: limit.parse().context("invalid event limit")?,
                },
                &CancellationToken::new(),
            )?;
            if jsonl {
                println!("{}", slotpilot::render_jsonl(&response)?);
            } else {
                match response.outcome {
                    slotpilot_api::SubscriptionOutcome::Events { events, .. } => {
                        for event in events {
                            println!("{}", slotpilot::render_event(&event));
                        }
                    }
                    _ => println!("{}", serde_json::to_string(&response)?),
                }
            }
            return Ok(());
        }
        _ => bail!(
            "usage: slotpilot status|devices audio list|receive start|stop|status|history|events follow; RF operation is unavailable"
        ),
    };
    let address = EndpointAddress::for_user(runtime_directory, "phase0-dev")?;
    let mutating = matches!(command, Command::ReceiveStart { .. } | Command::ReceiveStop);
    let request_id = match (mutating, request_id) {
        (true, None) => bail!("receive start/stop requires --request-id"),
        (_, Some(value)) => value,
        (false, None) => "req_phase2read"
            .parse()
            .context("invalid built-in request identity")?,
    };
    let response =
        slotpilot::request_command(&address, request_id, command, &CancellationToken::new())?;
    if json {
        println!("{}", slotpilot::render_json(&response)?);
    } else {
        println!("{}", slotpilot::render_table(&response));
    }
    Ok(())
}

fn remove_flag(arguments: &mut Vec<String>, flag: &str) -> bool {
    let Some(index) = arguments.iter().position(|value| value == flag) else {
        return false;
    };
    arguments.remove(index);
    true
}

fn remove_option(arguments: &mut Vec<String>, option: &str) -> Option<String> {
    let index = arguments.iter().position(|value| value == option)?;
    arguments.remove(index);
    (index < arguments.len()).then(|| arguments.remove(index))
}

fn parse_platform(value: &str) -> anyhow::Result<slotpilot_api::InputPlatform> {
    match value {
        "macos_core_audio" | "mac_os_core_audio" => {
            Ok(slotpilot_api::InputPlatform::MacOsCoreAudio)
        }
        "windows_wasapi" => Ok(slotpilot_api::InputPlatform::WindowsWasapi),
        "linux_alsa" => Ok(slotpilot_api::InputPlatform::LinuxAlsa),
        "linux_jack" => Ok(slotpilot_api::InputPlatform::LinuxJack),
        _ => anyhow::bail!("invalid input platform"),
    }
}

fn parse_sample_format(value: &str) -> anyhow::Result<slotpilot_api::InputSampleFormat> {
    match value {
        "signed8" => Ok(slotpilot_api::InputSampleFormat::Signed8),
        "signed16" => Ok(slotpilot_api::InputSampleFormat::Signed16),
        "signed24" => Ok(slotpilot_api::InputSampleFormat::Signed24),
        "signed32" => Ok(slotpilot_api::InputSampleFormat::Signed32),
        "signed64" => Ok(slotpilot_api::InputSampleFormat::Signed64),
        "unsigned8" => Ok(slotpilot_api::InputSampleFormat::Unsigned8),
        "unsigned16" => Ok(slotpilot_api::InputSampleFormat::Unsigned16),
        "unsigned24" => Ok(slotpilot_api::InputSampleFormat::Unsigned24),
        "unsigned32" => Ok(slotpilot_api::InputSampleFormat::Unsigned32),
        "unsigned64" => Ok(slotpilot_api::InputSampleFormat::Unsigned64),
        "float32" => Ok(slotpilot_api::InputSampleFormat::Float32),
        "float64" => Ok(slotpilot_api::InputSampleFormat::Float64),
        _ => anyhow::bail!("unsupported sample format"),
    }
}
