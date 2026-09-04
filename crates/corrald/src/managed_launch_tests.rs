use super::*;

use corral_core::NativeResumeEligibility;

/// Every refusal a person can be shown, with the enum itself as the anchor.
///
/// The match below is why this exists: adding an arm to `ResumeRefused` stops
/// it compiling, so a new refusal has to be named here before the tests that
/// police what refusals say can run at all.
fn every_refusal() -> Vec<ResumeRefused> {
    let refusals = vec![
        ResumeRefused::DirectoryUnknown,
        ResumeRefused::IdentityUnknown,
        ResumeRefused::Eligibility(NativeResumeEligibility::IdentityContested),
        ResumeRefused::Eligibility(NativeResumeEligibility::AssuranceTooWeak),
        ResumeRefused::Eligibility(NativeResumeEligibility::Eligible),
        ResumeRefused::UnknownProvider("codex".to_owned()),
        ResumeRefused::RunStillLive,
        ResumeRefused::EndUnverifiable,
        ResumeRefused::NoPreviousRun,
        ResumeRefused::EpisodeOrderUnknown,
    ];
    for refusal in &refusals {
        match refusal {
            ResumeRefused::DirectoryUnknown
            | ResumeRefused::IdentityUnknown
            | ResumeRefused::UnknownProvider(_)
            | ResumeRefused::RunStillLive
            | ResumeRefused::EndUnverifiable
            | ResumeRefused::NoPreviousRun
            | ResumeRefused::EpisodeOrderUnknown => {}
            ResumeRefused::Eligibility(eligibility) => match eligibility {
                NativeResumeEligibility::Eligible
                | NativeResumeEligibility::AssuranceTooWeak
                | NativeResumeEligibility::IdentityContested => {}
            },
        }
    }
    refusals
}

/// The terminology law, enforced where it is easiest to break: every refusal a
/// person reads names facts and actions, never Corral's machinery
/// (`PRODUCT.md` §8).
///
/// Session is the one domain noun a person is shown.
#[test]
fn no_continuation_refusal_exposes_corral_machinery() {
    let refusals = every_refusal();
    for refusal in refusals {
        let said = refusal.to_string().to_lowercase();
        for jargon in [
            "binding",
            "assurance",
            "evidence",
            "contested",
            "token",
            "attested",
            "heuristic",
            "deterministic",
            "eligibility",
        ] {
            assert!(!said.contains(jargon), "{said:?} exposes {jargon}");
        }
        assert!(!said.is_empty());
    }
}

/// `Run` is internal vocabulary, and it survives in exactly one sentence: the
/// one the founder fixed verbatim (grill Q7), where rewording it would be a
/// different promise about what Corral checked. Asserted rather than claimed
/// in a comment, so the next refusal that reaches for the word has to argue
/// with a test.
#[test]
fn the_word_run_reaches_a_person_in_one_refusal_only() {
    let verbatim = "Corral cannot verify that the previous run has exited, so it will not \
                    resume this provider session automatically";
    assert_eq!(ResumeRefused::EndUnverifiable.to_string(), verbatim);

    let says_run = |refusal: &ResumeRefused| {
        refusal
            .to_string()
            .split(|character: char| !character.is_alphanumeric())
            .any(|word| word.eq_ignore_ascii_case("run") || word.eq_ignore_ascii_case("runs"))
    };
    let refusals = every_refusal();
    assert_eq!(
        refusals.iter().filter(|refusal| says_run(refusal)).count(),
        1,
        "{:?}",
        refusals
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>(),
    );
}
