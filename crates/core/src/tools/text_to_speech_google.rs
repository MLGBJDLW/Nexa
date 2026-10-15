use super::*;
use crate::google_audio;

pub(super) async fn synthesize(
    client: &reqwest::Client,
    config: &TextToSpeechConfig,
    text: &str,
    model: &str,
    voice: &str,
) -> Result<GeneratedSpeech, CoreError> {
    if config.output_format != "wav" {
        return Err(CoreError::InvalidInput(
            "Google speech output uses WAV".into(),
        ));
    }
    let response = client
        .post(google_audio::endpoint(&base_url(config)))
        .header("x-goog-api-key", config.api_key.trim())
        .json(&google_audio::synthesis_request(model, text, voice))
        .send()
        .await
        .map_err(|error| {
            CoreError::TransientLlm(format!("Google speech request failed: {error}"))
        })?;
    let status = response.status();
    let bytes =
        read_bounded_response(response, "Google Speech", MAX_MINIMAX_RESPONSE_BYTES).await?;
    if !status.is_success() {
        return Err(CoreError::Llm(format!(
            "Google Speech returned {status}: {}",
            String::from_utf8_lossy(&bytes)
                .chars()
                .take(500)
                .collect::<String>()
        )));
    }
    decode(&serde_json::from_slice(&bytes)?)
}

fn decode(value: &Value) -> Result<GeneratedSpeech, CoreError> {
    let mut wavs = Vec::new();
    let mut total = 0usize;
    for block in google_audio::output_content(value, "audio")? {
        if !matches!(
            block["mime_type"].as_str(),
            Some("audio/wav" | "audio/x-wav")
        ) {
            return Err(CoreError::Parse(
                "Google Speech did not return the requested WAV format".into(),
            ));
        }
        let encoded = block["data"]
            .as_str()
            .ok_or_else(|| CoreError::Parse("Google Speech audio data is missing".into()))?;
        let bytes = decode_base64_audio(encoded, "Google Speech")?;
        total = total.saturating_add(bytes.len());
        if total > MAX_GENERATED_AUDIO_BYTES {
            return Err(CoreError::Parse(
                "Google Speech response exceeds its audio envelope".into(),
            ));
        }
        wavs.push(bytes);
    }
    let mut output = std::io::Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 24_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    {
        let mut writer = hound::WavWriter::new(&mut output, spec)
            .map_err(|e| CoreError::Parse(e.to_string()))?;
        for wav in &wavs {
            let mut reader = hound::WavReader::new(std::io::Cursor::new(wav))
                .map_err(|e| CoreError::Parse(format!("Invalid Google WAV: {e}")))?;
            if reader.spec() != spec {
                return Err(CoreError::Parse(
                    "Google WAV format does not match mono 24 kHz PCM16".into(),
                ));
            }
            for sample in reader.samples::<i16>() {
                writer
                    .write_sample(sample.map_err(|e| CoreError::Parse(e.to_string()))?)
                    .map_err(|e| CoreError::Parse(e.to_string()))?;
            }
        }
        writer
            .finalize()
            .map_err(|e| CoreError::Parse(e.to_string()))?;
    }
    // A single complete upstream WAV stays byte-identical. Multiple outputs
    // are joined as PCM, never as nested/concatenated RIFF containers.
    let bytes = if wavs.len() == 1 {
        wavs.remove(0)
    } else {
        output.into_inner()
    };
    Ok(GeneratedSpeech {
        bytes,
        media_type: "audio/wav".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wav(samples: &[i16]) -> Vec<u8> {
        let mut data = std::io::Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(
                &mut data,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 24_000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            for sample in samples {
                writer.write_sample(*sample).unwrap();
            }
            writer.finalize().unwrap();
        }
        data.into_inner()
    }
    #[test]
    fn native_wav_is_not_wrapped_twice_and_multiple_steps_join_pcm() {
        let first = wav(&[1, 2]);
        let block = |bytes: &[u8]| json!({"type":"audio","mime_type":"audio/wav","data":BASE64_STANDARD.encode(bytes)});
        let single = json!({"status":"completed","steps":[{"type":"model_output","content":[block(&first)]}]});
        assert_eq!(decode(&single).unwrap().bytes, first);
        let multiple = json!({"status":"completed","steps":[{"type":"model_output","content":[block(&first)]},{"type":"model_output","content":[block(&wav(&[3]))]}]});
        let output = decode(&multiple).unwrap();
        let mut reader = hound::WavReader::new(std::io::Cursor::new(output.bytes)).unwrap();
        assert_eq!(
            reader
                .samples::<i16>()
                .map(Result::unwrap)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert!(decode(&json!({"status":"failed"})).is_err());
        assert!(decode(&json!({"status":"completed","steps":[{"type":"model_output","content":[block(b"bad")]}]})).is_err());
    }
}
