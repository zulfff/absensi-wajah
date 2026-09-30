# Design Brief — Absensi Wajah

## 1. Product identity

Attendance system for an Indonesian school. A kiosk (browser + camera + screen)
recognises students' faces; an admin web dashboard manages students, face
enrollment, devices, and attendance records.

**Two surfaces, one product:**
- **Kiosk** — full-screen, one job: recognise a face and confirm. Used by
  students, standing, 2-3 seconds per person, sometimes in a noisy hallway.
- **Admin dashboard** — used by a school operator/teacher at a desk, making
  decisions about enrollment quality and attendance records.

## 2. Design intent

**Style:** Minimal Clean with a functional accent. Hairline separators, mostly
neutral, one teal accent doing real work. Data-forward: numbers are large and
undecorated, tables carry proportion context, status is a 6px dot plus a word.

**Anti-slop posture (this project's specific risks):**
- Do NOT wrap every section in a rounded card. Data regions are flush panels
  with a hairline top rule (`.panel`), not floating cards.
- Do NOT build a uniform grid of identical metric cards. The dashboard hero is
  a single dominant figure (a large count + hourly sparkline) on the canvas,
  with a compact inline stat strip beside it.
- Numbers use `tabular-nums`. Comparisons are plain secondary text.
- No emoji, ever. Icons are inline Lucide paths (`AppIcon.vue`).
- Status is a dot + word, never a coloured pill badge.

**Voice:** Indonesian, direct, warm, no filler. Verbs name the outcome
("Aktifkan wajah", "Simpan pendaftaran", "Hapus filter"), never "OK"/"Submit".
Error messages say what failed and what to do next.

## 3. Audience

- **Operator (primary):** school staff, not technical. Needs the enrollment
  quality feedback to be legible without ML knowledge — hence the plain-language
  guidance ("Terlalu jauh. Mendekatlah ke kamera.") and the explicit
  duplicate/risky-separation warnings in words, not scores alone.
- **Student (kiosk):** uses the kiosk for seconds, reads one large line. Must
  work with a face full of motion and no instruction reading.

## 4. Constraints

- Camera (`getUserMedia`) requires a **secure context** — HTTPS or localhost.
- Kiosk may be a weak device: the browser only captures and displays; all
  inference is server-side.
- The kiosk overlay sits on **arbitrary camera imagery**, so its palette is
  theme-independent (`--overlay-*` tokens) and must stay high-contrast.
- Fail-closed UX: when recognition is unsure, the kiosk never guesses a name;
  it shows a retry prompt or asks for teacher help.

## 5. Design principles (ranked)

1. **Value first** — every screen answers one operator question. The dashboard
   answers "did attendance work today?"; the enroll view answers "is this face
   captured well enough to trust?".
2. **Honest states** — loading, empty (first-run vs filtered), error, and
   partial all have explicit designs. The dashboard shows a reject ratio, not
   just the good number.
3. **Token discipline** — no raw hex, off-scale spacing, or magic z-index
   outside `tokens.css`. Light and dark are both designed.
4. **Accessibility floor** — focus-visible everywhere, semantic landmarks, 4.5:1
   text contrast, reduced-motion honored, tables reflow via a scroll container.

## 6. Learned constraints

- **Never build a 4-up identical metric-card grid on the dashboard.** The
  dashboard must have one dominant figure. (Corrected after the first build
  shipped a uniform card row — a generated-UI tell.)
- **The table is the product on list screens.** It sits on a flush `.panel`,
  not inside a floating `.card`.
- **Mobile is a real layout, not a squashed desktop.** Below 768px the sidebar is
  replaced by a bottom nav in the thumb zone; theme and logout move to a fixed
  top bar (destructive action out of the thumb path). Never ship the 3.5rem icon
  rail to a phone — it is a tablet affordance.
- **Every fixed edge gets its safe-area inset.** A bottom nav or kiosk bar that
  ignores `env(safe-area-inset-bottom)` sits under the home indicator.
- **Use `dvh`, not `vh`.** A `100vh` full-screen layout is clipped by the mobile
  browser chrome.
- **Do not claim visual verification from scores.** The build agent has no image
  input; anti-slop/token/a11y scores are objective but are not a substitute for
  a human looking at the render. The `check_fold` tool can only see public routes
  (login, kiosk) — the admin dashboard redirects a fresh browser to login, so it
  has never been rendered for that tool. Responsive claims here come from a
  headless-Chromium DOM harness (`scrollWidth == clientWidth`, computed nav
  display, measured touch-target heights), not from looking at pixels.

## 7. Craft Read (recurring)

> **Craft Read:** school attendance console for a school operator, product
> language, Graphite + teal accent, variance 4, signature bet: the "today so far"
> hero — a large live check-in count with an hourly sparkline, flush on the
> canvas, with secondary stats as an inline strip rather than another card row.
