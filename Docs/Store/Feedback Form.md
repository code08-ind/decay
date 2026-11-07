# Decay beta — feedback form (Google Forms)

> Create in Google Forms. Settings: collect email addresses **on** (to match exports and book calls); "Limit to 1 response" **off** (testers may report problems mid-beta). Add a **File upload** question for the metrics file (needs testers to sign in to Google; if that's a problem, ask them to email the file instead).

**Form title:** Decay beta: feedback
**Description:** Takes about 5 minutes. Answers are used only to improve Decay and are deleted when the beta ends. Privacy: https://www.aryangarg.com/decay/privacy-policy

## Section 1: About you

1. **Which browser do you use Decay in?** *(multiple choice)* Chrome · Edge · Brave · Arc · Other
2. **On a normal day, how many tabs do you have open?** *(multiple choice)* Under 10 · 10–30 · 30–60 · 60–100 · Over 100
3. **What's your work?** *(multiple choice)* Software engineering · Design · Product / PM · Research / writing · Student · Other

## Section 2: How it went

4. **Is automatic mode still on?** *(multiple choice)* Yes · No, I switched back to shadow mode · No, I'm about to uninstall it · I never turned it on
4b. **On what date did you turn on automatic mode?** *(date, optional)*
5. **If it's off: what made you turn it off?** *(paragraph, optional)*
6. **Did Decay ever archive a tab you were still using?** *(multiple choice)* Never · Once or twice · A few times a week · Often
7. **When you needed an archived tab back, how easy was it to find?** *(linear scale 1–5, "Very hard" → "Instant")*
8. **Has your number of open tabs changed?** *(multiple choice)* Much fewer · A bit fewer · About the same · More
9. **How did it feel to have tabs closed for you?** *(multiple choice)* Relieved · Fine, didn't notice · Slightly anxious · Very anxious
10. **What was the best part?** *(paragraph)*
11. **What was the most annoying or confusing part?** *(paragraph)*
12. **Any bugs?** What happened, and roughly when? *(paragraph, optional)*

## Section 3: Metrics file

13. **Upload your metrics file** (Insights → Share with beta team → Save file; named `decay-beta-metrics-YYYY-MM-DD.json`). *(file upload, 1 file, JSON/any, max 10 MB, optional)*

## Section 4: The desktop app (pre-order ask, P0.10)

> Decay's next step is a macOS and Windows desktop app that does the same for app windows, as well as tabs across all your browsers: one hotkey to search everything you've put away. It would be a one-time purchase (no subscription), with the browser extension staying free.

14. **How disappointed would you be if you could no longer use Decay?** *(multiple choice)* Very disappointed · Somewhat disappointed · Not disappointed
15. **Would you pre-order the desktop app at a launch price of $19?** *(multiple choice)* Yes, send me the pre-order link · Maybe, at a lower price · No
16. **If "maybe": what price would feel fair?** *(short answer, optional)*
17. **Would you do a 20-minute call about your experience?** *(multiple choice)* Yes (I'll email you to schedule) · No thanks

---

### Gate 0 read-out from the responses

- **Adoption (primary, from the files):** testers whose file has `setup.automaticForDays ≥ 14` ÷ all testers who installed → **≥ 30%**. Report it over all testers, not only those who turned automatic mode on, so non-adoption counts. Q4 / Q4b are the cross-check, and cover testers who sent no file.
- Regret from the files: Σ `restoredWithin24h` ÷ Σ `archived`, pooled over testers → **≤ 15%**.
- Q15 = "Yes" → **≥ 5 would pre-order** (stated intent, PRD §9). Real pre-orders are collected in P0.10.
