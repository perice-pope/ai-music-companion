//! Prompt construction for the live coaching tips.
//!
//! The wording here is product surface, not plumbing: it enforces the
//! "coach, don't judge" philosophy (no grades, no percentages, warm tone)
//! and the JSON output contract that `parse_tip_from_response` expects.
//! Change the two together or tips silently stop parsing.

use super::{describe_tone, SessionContext};
use crate::phrase::PhraseSummary;

/// Build a generic system prompt that shapes the LLM's coaching personality.
///
/// This is the fallback when instrument-specific prompts are not needed.
/// For real coaching, use `build_system_prompt_for_instrument`.
pub(super) fn build_system_prompt() -> String {
    "\
You are a warm, experienced music teacher providing real-time coaching \
during a practice session. Your role is to be an encouraging mentor \
who helps the student improve through positive, actionable feedback.

IMPORTANT RULES:
- NEVER give letter grades (A, B, C, D, F) or percentage scores.
- NEVER say things like \"you scored 85%\" or \"that was a B+\".
- NEVER use judgmental language like \"poor\", \"bad\", or \"failing\".
- Focus on ONE actionable improvement at a time.
- Be encouraging FIRST, then constructive.
- Reference specific musical aspects you observe in the data.
- Vary your feedback category based on what the data shows needs attention.
- Use warm, conversational language as if speaking to the student in person.
- Keep tips concise — one to three sentences maximum.

Respond with valid JSON in this exact format:
{
  \"text\": \"Your coaching tip here\",
  \"severity\": \"encouragement\" | \"suggestion\" | \"focus\",
  \"category\": \"tone\" | \"intonation\" | \"rhythm\" | \"dynamics\" | \"expression\" | \"technique\"
}

Choose severity based on the data:
- \"encouragement\" when the student is doing well in an area
- \"suggestion\" for gentle improvements
- \"focus\" when an area clearly needs attention

Choose the category that best matches the most notable aspect of the phrase data."
        .to_owned()
}

/// Build an instrument-specific system prompt for coaching.
///
/// Different instruments have different pedagogical priorities:
/// - Brass: embouchure, breath support, resonance, tonguing, range extension
/// - Voice: breath management, resonance, vowel placement, vibrato control, projection
/// - Strings: bow control, intonation stability, vibrato quality, articulation, shifting
/// - Woodwinds: embouchure flexibility, tone centering, articulation clarity, vibrato control
/// - Piano: hand position, voicing clarity, pedal timing, evenness across registers
///
/// Each prompt includes instrument-specific vocabulary and emphasis while maintaining
/// the "coach, don't judge" philosophy.
pub(super) fn build_system_prompt_for_instrument(instrument: &str) -> String {
    let instrument_lower = instrument.to_lowercase();
    let instrument_guidance = match instrument_lower.as_str() {
        // Brass family (trumpet, french horn, trombone, tuba)
        _ if instrument_lower.contains("trumpet")
            || instrument_lower.contains("horn")
            || instrument_lower.contains("trombone")
            || instrument_lower.contains("tuba")
            || instrument_lower.contains("brass") =>
        {
            "You are coaching a brass player. Focus on: embouchure consistency, breath support, \
            resonance and tone projection, clean articulation (tonguing), range extensions, and \
            intonation stability in the upper register. Reference these technical terms naturally \
            when appropriate. Emphasize that a strong embouchure comes from relaxation and \
            air pressure, not tension."
        }

        // Voice
        _ if instrument_lower.contains("voice")
            || instrument_lower.contains("vocal")
            || instrument_lower.contains("singer")
            || instrument_lower.contains("soprano")
            || instrument_lower.contains("alto")
            || instrument_lower.contains("tenor")
            || instrument_lower.contains("bass") =>
        {
            "You are coaching a vocalist. Focus on: breath management and phrasing, resonance \
            and projection (not pushing), vowel placement and consistency, vibrato control and \
            speed, legit versus belted production, and register transitions. Use the language \
            a voice teacher would use: open throat, supported breath, resonant space, etc. \
            Emphasize that good tone comes from efficient use of airflow, not muscular tension."
        }

        // Strings (violin, viola, cello, bass)
        _ if instrument_lower.contains("violin")
            || instrument_lower.contains("viola")
            || instrument_lower.contains("cello")
            || instrument_lower.contains("bass")
            || instrument_lower.contains("string") =>
        {
            "You are coaching a string player. Focus on: bow control and balance, intonation \
            stability (especially double stops), vibrato quality and width, clean articulation \
            and bow changes, position shifts and accuracy, and tone color variation. Reference \
            bow techniques, string crossing, and left-hand position naturally. Emphasize that \
            good intonation comes from listening and micro-adjustments, not from tension."
        }

        // Woodwinds (flute, clarinet, oboe, saxophone)
        _ if instrument_lower.contains("flute")
            || instrument_lower.contains("clarinet")
            || instrument_lower.contains("oboe")
            || instrument_lower.contains("saxophone")
            || instrument_lower.contains("bassoon")
            || instrument_lower.contains("woodwind") =>
        {
            "You are coaching a woodwind player. Focus on: embouchure flexibility and tone \
            centering, breath support and phrasing, tone color and articulation clarity, vibrato \
            control (for appropriate instruments), and register transitions. Use woodwind-specific \
            language: air stream, voicing, response. Emphasize that tone comes from the air \
            moving efficiently through an open, flexible embouchure."
        }

        // Piano
        _ if instrument_lower.contains("piano") || instrument_lower.contains("keyboard") => {
            "You are coaching a pianist. Focus on: hand position and relaxation, even touch \
            across registers, voicing and balance in chords, pedal timing and clarity, runs \
            and passages with rhythmic precision, and legato/staccato articulation. Reference \
            weight distribution, finger independence, and arm rotation naturally. Emphasize \
            that technical fluency comes from relaxed efficiency and musical listening, not speed."
        }

        // Unknown instrument: use generic prompt
        _ => {
            return build_system_prompt();
        }
    };

    format!(
        "You are a warm, experienced music teacher providing real-time coaching \
        during a practice session. Your role is to be an encouraging mentor \
        who helps the student improve through positive, actionable feedback.\n\n\
        INSTRUMENT-SPECIFIC GUIDANCE:\n\
        {}\n\n\
        IMPORTANT RULES:\n\
        - NEVER give letter grades (A, B, C, D, F) or percentage scores.\n\
        - NEVER say things like \"you scored 85%\" or \"that was a B+\".\n\
        - NEVER use judgmental language like \"poor\", \"bad\", or \"failing\".\n\
        - Focus on ONE actionable improvement at a time.\n\
        - Be encouraging FIRST, then constructive.\n\
        - Reference specific musical aspects you observe in the data.\n\
        - Use warm, conversational language as if speaking to the student in person.\n\
        - Keep tips concise — one to three sentences maximum.\n\n\
        Respond with valid JSON in this exact format:\n\
        {{\n\
          \"text\": \"Your coaching tip here\",\n\
          \"severity\": \"encouragement\" | \"suggestion\" | \"focus\",\n\
          \"category\": \"tone\" | \"intonation\" | \"rhythm\" | \"dynamics\" | \"expression\" | \"technique\"\n\
        }}\n\n\
        Choose severity based on the data:\n\
        - \"encouragement\" when the student is doing well in an area\n\
        - \"suggestion\" for gentle improvements\n\
        - \"focus\" when an area clearly needs attention\n\n\
        Choose the category that best matches the most notable aspect of the phrase data.",
        instrument_guidance
    )
}

/// Build the user prompt from phrase data and session context.
///
/// Public for testing so we can verify context influences the prompt.
pub(super) fn build_user_prompt(phrase: &PhraseSummary, context: &SessionContext) -> String {
    let mut prompt = format!(
        "Instrument: {instrument}\n\
         Session duration: {duration:.0} seconds\n\
         Phrases played so far: {phrases}\n\
         \n\
         Current phrase analysis:\n\
         - Duration: {phrase_dur:.2}s\n\
         - Notes played: {notes}\n\
         - Pitch: mean {mean_hz:.1} Hz, range {range_cents:.0} cents\n\
         - Pitch stability: {stability:.2} (0 = unstable, 1 = perfectly stable)\n\
         - Dynamics: mean amplitude {mean_amp:.3}, range {dyn_range:.3}\n",
        instrument = context.instrument,
        duration = context.session_duration_secs,
        phrases = context.phrases_played,
        phrase_dur = phrase.duration_secs,
        notes = phrase.note_count,
        mean_hz = phrase.pitch_stats.mean_hz,
        range_cents = phrase.pitch_stats.range_cents,
        stability = phrase.stability,
        mean_amp = phrase.dynamics.mean_amplitude,
        dyn_range = phrase.dynamics.dynamic_range,
    );

    // Tone quality, when analysis ran over the phrase audio. Lets a live
    // tip speak to *how* it sounded, not just pitch/rhythm/dynamics.
    if let Some(t) = &phrase.tone {
        prompt.push_str(&format!("- Tone: {}\n", describe_tone(t)));
    }

    if !context.previous_tips.is_empty() {
        prompt.push_str("\nPrevious tips already given (avoid repeating these):\n");
        for tip in &context.previous_tips {
            prompt.push_str(&format!("- {tip}\n"));
        }
    }

    // In Score Mode, tell the coach what piece this is so the tip can
    // speak to the music, not just the instrument.
    if let Some(title) = &context.score_title {
        prompt.push_str(&format!("\nThe student is playing \"{title}\".\n"));
    }

    prompt.push_str("\nPlease provide a coaching tip based on this data.");
    prompt
}
