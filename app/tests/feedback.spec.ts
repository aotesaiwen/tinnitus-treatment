import { expect, test, type Page } from "@playwright/test";

async function feedbackFixture(page: Page, study = false) {
  await page.addInitScript(({ study }) => {
    const state = {
      setup_done: true, participant_id: study ? "P012" : "self",
      daily_goal_min: 60, today_min: 12, week: [{ date: "2026-10-21", minutes: 12 }],
      player: { state: "idle", can_rate: false, track_id: null, title: null, position_s: 0, duration_s: 0 },
      music_preferences: { provider: "composer", style: "drift", duration_s: 60, tempo_bpm: 54, prompt: "" },
      library: [
        { id: "track-42", title: "Slow drift · 42", duration_s: 60, favorite: false,
          recipe: study ? null : { provider: "composer", request: { style: "drift", duration_s: 60, tempo_bpm: 54, seed: 42, prompt: "" } } },
        { id: "track-7", title: "Slow drift · 7", duration_s: 60, favorite: false, recipe: null },
      ],
      volume_db: -6, library_dir: "/fixture/library",
      clinician_unlocked: false, pin_set: true, device_fs: 48000, matching_active: false,
      error: null, generation: { stage: "", title: null, track_id: null, message: null }, can_generate: !study,
    };
    const fixture = {
      state, calls: [] as { cmd: string; args: unknown }[],
      feedbackError: null as string | null,
      holdFeedback: false,
      releaseFeedback: null as (() => void) | null,
    };
    Object.assign(window, {
      __feedbackFixture: fixture,
      __TAURI_INTERNALS__: {
        invoke: async (cmd: string, args: any) => {
          fixture.calls.push({ cmd, args });
          if (cmd === "status") return structuredClone(state);
          if (cmd === "player_play") {
            const track = state.library.find(track => track.id === args.trackId)!;
            Object.assign(state.player, {
              state: "playing", can_rate: track.id === "track-42",
              track_id: track.id, title: track.title, duration_s: track.duration_s, position_s: 10,
            });
          }
          if (cmd === "player_toggle") state.player.state = state.player.state === "playing" ? "paused" : "playing";
          if (cmd === "music_feedback_save") {
            if (fixture.feedbackError) throw fixture.feedbackError;
            if (fixture.holdFeedback) return new Promise<void>(resolve => { fixture.releaseFeedback = resolve; });
          }
          return null;
        },
      },
    });
  }, { study });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Music", exact: true })).toBeVisible();
}

async function startTrack(page: Page) {
  await page.getByRole("button", { name: "Play Slow drift · 42", exact: true }).click();
  await expect(page.getByRole("heading", { name: "How was this music?" })).toBeVisible();
}

test("ratings appear after playback and keep unanswered discomfort distinct from No", async ({ page }) => {
  await feedbackFixture(page);
  await expect(page.getByRole("heading", { name: "How was this music?" })).toHaveCount(0);
  await startTrack(page);
  await expect(page.getByRole("button", { name: "Save rating", exact: true })).toBeDisabled();
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await page.getByRole("combobox", { name: "Music preference", exact: true }).selectOption("4");
  await page.getByRole("button", { name: "Save rating", exact: true }).click();
  await expect(page.getByRole("status").filter({ hasText: "Your rating was saved on this computer." })).toHaveText("Your rating was saved on this computer.");
  await expect(page.getByRole("button", { name: "Rating saved", exact: true })).toBeDisabled();
  const calls = await page.evaluate(() => (window as any).__feedbackFixture.calls);
  expect(calls.filter((call: any) => call.cmd === "music_feedback_save").map((call: any) => call.args)).toEqual([
    { feedback: { participant_id: "self", track_id: "track-42", rating: 4, discomfort: null } },
  ]);
  expect(calls.filter((call: any) => ["player_play", "generation_start", "profile_save"].includes(call.cmd)))
    .toEqual([{ cmd: "player_play", args: { trackId: "track-42" } }]);
});

test("study listeners can report discomfort without exposing recipes or enabling creation", async ({ page }) => {
  await feedbackFixture(page, true);
  await startTrack(page);
  await page.getByRole("combobox", { name: "Music preference", exact: true }).selectOption("2");
  await page.getByRole("combobox", { name: "Did this music feel uncomfortable? (optional)", exact: true }).selectOption("yes");
  await page.getByRole("button", { name: "Save rating", exact: true }).click();
  await expect(page.getByRole("status").filter({ hasText: "Your rating was saved on this computer." })).toHaveText("Your rating was saved on this computer.");
  await expect(page.getByRole("button", { name: "Compose my music" })).toBeDisabled();
  await expect(page.getByRole("button", { name: /Reuse settings/ })).toHaveCount(0);
  await expect(page.locator("body")).not.toContainText("sham");
  expect(await page.evaluate(() => (window as any).__feedbackFixture.calls.find((call: any) => call.cmd === "music_feedback_save").args))
    .toEqual({ feedback: { participant_id: "P012", track_id: "track-42", rating: 2, discomfort: true } });
});

test("a failed save preserves answers for explicit retry and does not replay the track", async ({ page }) => {
  await feedbackFixture(page);
  await startTrack(page);
  await page.getByRole("combobox", { name: "Music preference", exact: true }).selectOption("5");
  await page.getByRole("combobox", { name: "Did this music feel uncomfortable? (optional)", exact: true }).selectOption("no");
  await page.evaluate(() => { (window as any).__feedbackFixture.feedbackError = "Could not save music feedback: disk full"; });
  await page.getByRole("button", { name: "Save rating", exact: true }).click();
  await expect(page.getByRole("alert")).toHaveText("Could not save music feedback: disk full");
  await expect(page.getByRole("combobox", { name: "Music preference", exact: true })).toHaveValue("5");
  await expect(page.getByRole("combobox", { name: "Did this music feel uncomfortable? (optional)", exact: true })).toHaveValue("no");
  await page.evaluate(() => { (window as any).__feedbackFixture.feedbackError = null; });
  await page.getByRole("button", { name: "Save rating", exact: true }).click();
  await expect(page.getByRole("button", { name: "Rating saved", exact: true })).toBeDisabled();
  expect(await page.evaluate(() => (window as any).__feedbackFixture.calls.filter((call: any) => call.cmd === "player_play").length)).toBe(1);
  expect(await page.evaluate(() => (window as any).__feedbackFixture.calls.filter((call: any) => call.cmd === "music_feedback_save").at(-1).args.feedback.discomfort)).toBe(false);
});

test("track changes discard an old draft and idle or imported tracks cannot be rated", async ({ page }) => {
  await feedbackFixture(page);
  await startTrack(page);
  await page.getByRole("combobox", { name: "Music preference", exact: true }).selectOption("1");
  await page.getByRole("button", { name: "Play Slow drift · 7", exact: true }).click();
  await expect(page.getByRole("heading", { name: "How was this music?" })).toHaveCount(0);
  await startTrack(page);
  await expect(page.getByRole("combobox", { name: "Music preference", exact: true })).toHaveValue("");
  await page.evaluate(() => {
    Object.assign((window as any).__feedbackFixture.state.player, { state: "idle", can_rate: false });
  });
  await expect(page.getByRole("heading", { name: "How was this music?" })).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__feedbackFixture.calls.some((call: any) => call.cmd === "music_feedback_save"))).toBe(false);
});

test("a delayed save for the previous participant cannot show success for the new listener", async ({ page }) => {
  await feedbackFixture(page);
  await startTrack(page);
  await page.getByRole("combobox", { name: "Music preference", exact: true }).selectOption("3");
  await page.evaluate(() => { (window as any).__feedbackFixture.holdFeedback = true; });
  await page.getByRole("button", { name: "Save rating", exact: true }).click();
  await expect(page.getByRole("button", { name: "Saving rating…" })).toBeDisabled();
  await page.evaluate(() => { (window as any).__feedbackFixture.state.participant_id = "P099"; });
  await expect(page.getByRole("combobox", { name: "Music preference", exact: true })).toHaveValue("");
  await page.evaluate(() => (window as any).__feedbackFixture.releaseFeedback());
  await expect(page.getByRole("button", { name: "Save rating", exact: true })).toBeDisabled();
  await expect(page.getByRole("status").filter({ hasText: "Your rating was saved on this computer." })).toHaveCount(0);
  expect(await page.evaluate(() => (window as any).__feedbackFixture.calls.filter((call: any) => call.cmd === "music_feedback_save").length)).toBe(1);
});

test("the feedback controls fit a narrow display in both themes", async ({ page }) => {
  await feedbackFixture(page);
  await startTrack(page);
  for (const colorScheme of ["light", "dark"] as const) {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.emulateMedia({ colorScheme, reducedMotion: "reduce" });
    await expect(page.getByRole("combobox", { name: "Music preference", exact: true })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  }
});
