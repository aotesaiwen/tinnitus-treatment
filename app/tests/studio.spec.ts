import { expect, test, type Page } from "@playwright/test";

async function desktopFixture(page: Page, study = false) {
  await page.addInitScript(({ study }) => {
    const state = {
      setup_done: true, participant_id: study ? "P012" : "self",
      daily_goal_min: 60, today_min: 12, week: [
        { date: "2026-10-14", minutes: 25 }, { date: "2026-10-15", minutes: 40 },
        { date: "2026-10-16", minutes: 15 }, { date: "2026-10-17", minutes: 60 },
        { date: "2026-10-18", minutes: 38 }, { date: "2026-10-19", minutes: 45 }, { date: "2026-10-20", minutes: 12 },
      ],
      player: { state: "idle", track_id: null, title: null, position_s: 0, duration_s: 0 },
      music_preferences: JSON.parse(localStorage.getItem("musicPreferences") ?? "null") ?? { provider: "composer", style: "drift", duration_s: 60, tempo_bpm: 54, prompt: "" },
      library: [{ id: "track-42", title: "Slow drift · 42", duration_s: 60, favorite: false,
        recipe: study ? null : { provider: "composer", request: { style: "drift", duration_s: 60, tempo_bpm: 54, seed: 42, prompt: "" } } }],
      volume_db: -6, library_dir: "/fixture/library",
      clinician_unlocked: false, pin_set: true, device_fs: 48000, matching_active: false,
      error: null, generation: { stage: "", title: null, message: null }, can_generate: !study,
    };
    const fixture = {
      state, calls: [] as { cmd: string; args: unknown }[], holdNextStatus: false,
      releaseStatus: null as (() => void) | null,
      savePath: "/fixture/export/slow-drift_listening.wav" as string | null,
      exportError: null as string | null,
    };
    Object.assign(window, {
      __studioFixture: fixture,
      __TAURI_INTERNALS__: {
        invoke: async (cmd: string, args: unknown) => {
          fixture.calls.push({ cmd, args });
          if (cmd === "plugin:dialog|save") return fixture.savePath;
          if (cmd === "library_export") {
            if (fixture.exportError) throw fixture.exportError;
            return { path: (args as any).path, report_file: "export.json" };
          }
          if (cmd === "music_preferences_save") {
            state.music_preferences = (args as any).preferences;
            localStorage.setItem("musicPreferences", JSON.stringify(state.music_preferences));
          }
          if (cmd === "library_favorite") {
            const track = state.library.find(t => t.id === (args as any).trackId);
            if (track) track.favorite = (args as any).favorite;
          }
          if (cmd === "status") {
            const snapshot = structuredClone(state);
            if (fixture.holdNextStatus) {
              fixture.holdNextStatus = false;
              return new Promise(resolve => { fixture.releaseStatus = () => resolve(snapshot); });
            }
            return snapshot;
          }
          if (cmd === "generation_start") state.generation.stage = "composing";
          if (cmd === "generation_cancel") state.generation.stage = "cancelled";
          if (cmd === "generation_health") throw "ACE-Step is offline. Start the service or use the offline composer.";
          return null;
        },
      },
    });
  }, { study });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "Music, made for your day." })).toBeVisible();
}

test("creates a reproducible variation, shows progress, and cancels", async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole("radio", { name: /Gentle focus/ }).check();
  await page.getByText("Recreate a variation", { exact: true }).click();
  await page.getByLabel(/Seed/).fill("123");
  await page.getByRole("button", { name: "Compose my music" }).click();
  await expect(page.getByRole("button", { name: "Creating your music" })).toBeDisabled();
  const request = await page.evaluate(() => (window as any).__studioFixture.calls.find((c: any) => c.cmd === "generation_start").args);
  expect(request).toEqual({ provider: "composer", request: { style: "focus", duration_s: 60, tempo_bpm: 76, seed: 123, prompt: "" } });
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(page.getByRole("button", { name: "Compose my music" })).toBeEnabled();
});

test("reports unavailable AI and submits the selected music description", async ({ page }) => {
  await desktopFixture(page);
  await page.getByLabel("Create with").selectOption("ace_step");
  await page.getByRole("button", { name: "Check AI connection" }).click();
  await expect(page.getByRole("status").filter({ hasText: "ACE-Step is offline" })).toBeVisible();
  await page.getByLabel("Describe your sound").fill("A gentle piano by the sea");
  await page.getByRole("button", { name: "Generate AI music" }).click();
  const request = await page.evaluate(() => (window as any).__studioFixture.calls.find((c: any) => c.cmd === "generation_start").args);
  expect(request.provider).toBe("ace_step");
  expect(request.request.prompt).toBe("A gentle piano by the sea");
});

test("failed checks allow a retry and successful creation updates the library", async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole("button", { name: "Compose my music" }).click();
  await page.evaluate(() => {
    (window as any).__studioFixture.state.generation = { stage: "failed", title: null, message: "This track did not pass the audio quality checks." };
  });
  await expect(page.getByText("This track did not pass the audio quality checks.")).toBeVisible();
  await page.getByRole("button", { name: "Compose my music" }).click();
  await page.evaluate(() => {
    const state = (window as any).__studioFixture.state;
    state.generation = { stage: "ready", title: "Slow drift", message: "Added to your library." };
    state.library.push({ id: "track-123", title: "Slow drift · 123", duration_s: 60, favorite: false, recipe: null });
  });
  await expect(page.getByText("Added to your library.")).toBeVisible();
  await page.getByRole("button", { name: "Play Slow drift · 123", exact: true }).click();
  expect(await page.evaluate(() => (window as any).__studioFixture.calls.find((c: any) => c.cmd === "player_play").args)).toEqual({ trackId: "track-123" });
});

test("retrying a failed fixed seed creates a fresh variation", async ({ page }) => {
  await desktopFixture(page);
  await page.getByText("Recreate a variation", { exact: true }).click();
  await page.getByLabel(/Seed/).fill("123");
  await page.evaluate(() => {
    (window as any).__studioFixture.state.generation = { stage: "failed", title: null, message: "Try a new variation or a different style." };
    Object.defineProperty(window.crypto, "getRandomValues", { value: (array: Uint32Array) => { array[0] = 456; return array; } });
  });
  await page.getByRole("button", { name: "Try a new variation", exact: true }).click();
  const request = await page.evaluate(() => (window as any).__studioFixture.calls.find((c: any) => c.cmd === "generation_start").args);
  expect(request.request.seed).toBe(456);
  await expect(page.getByLabel(/Seed/)).toHaveValue("");
});

test("plays a completed generation by its stable ID only after an explicit click", async ({ page }) => {
  await desktopFixture(page);
  await page.evaluate(() => {
    (window as any).__studioFixture.state.generation = {
      stage: "ready", title: "Slow drift · 42", track_id: "track-42", message: "Added to your library.",
    };
  });
  await expect(page.getByRole("button", { name: "Play new track" })).toBeVisible();
  expect(await page.evaluate(() => (window as any).__studioFixture.calls.some((c: any) => c.cmd === "player_play"))).toBe(false);
  await page.getByRole("button", { name: "Play new track" }).click();
  expect(await page.evaluate(() => (window as any).__studioFixture.calls.find((c: any) => c.cmd === "player_play").args)).toEqual({ trackId: "track-42" });
});

test("study participants cannot create tracks or see treatment assignments", async ({ page }) => {
  await desktopFixture(page, true);
  await expect(page.getByRole("button", { name: "Compose my music" })).toBeDisabled();
  await expect(page.getByText("Your clinician manages music creation for this study.")).toBeVisible();
  await expect(page.locator("body")).not.toContainText("sham");
  await expect(page.getByRole("button", { name: /Reuse settings/ })).toHaveCount(0);
  await expect(page.getByRole("button", { name: /Save .* as WAV/ })).toHaveCount(0);
  await page.evaluate(() => { (window as any).__studioFixture.state.generation.stage = "waiting"; });
  await expect(page.getByRole("button", { name: "Cancel", exact: true })).toBeDisabled();
});

test("saved music preferences survive reopening and do not overwrite a new draft during polling", async ({ page }) => {
  await desktopFixture(page);
  await page.getByRole("radio", { name: /Evening unwind/ }).check();
  await page.getByLabel("Track length").selectOption("120");
  await page.getByRole("button", { name: "Remember settings" }).click();
  await expect(page.getByText("Saved on this computer.")).toBeVisible();
  await page.reload();
  await expect(page.getByRole("radio", { name: /Evening unwind/ })).toBeChecked();
  await expect(page.getByLabel("Track length")).toHaveValue("120");
  await page.getByRole("radio", { name: /Gentle focus/ }).check();
  await page.waitForTimeout(1200);
  await expect(page.getByRole("radio", { name: /Gentle focus/ })).toBeChecked();
});

test("favorites filter and recipe reuse preserve the original variation without autoplay", async ({ page }) => {
  await desktopFixture(page);
  await page.getByLabel("Show tracks").selectOption("favorites");
  await expect(page.getByText("Favorite a track to keep it here.")).toBeVisible();
  await page.getByLabel("Show tracks").selectOption("all");
  await page.getByRole("button", { name: "Favorite Slow drift · 42", exact: true }).click();
  await expect(page.getByRole("button", { name: "Unfavorite Slow drift · 42" })).toHaveAttribute("aria-pressed", "true");
  await page.getByLabel("Show tracks").selectOption("favorites");
  await page.getByRole("button", { name: "Reuse settings for Slow drift · 42" }).click();
  await expect(page.getByLabel(/Seed/)).toHaveValue("42");
  await expect(page.getByText(/Creation settings restored/)).toBeVisible();
  expect(await page.evaluate(() => (window as any).__studioFixture.calls.some((c: any) => ["player_play", "generation_start"].includes(c.cmd)))).toBe(false);
  await page.getByLabel("Search your library").fill("missing track");
  await expect(page.getByText("No tracks match your search.")).toBeVisible();
});

test("a delayed status response cannot undo a newer favorite update", async ({ page }) => {
  await desktopFixture(page);
  await page.evaluate(() => { (window as any).__studioFixture.holdNextStatus = true; });
  await expect.poll(() => page.evaluate(() => typeof (window as any).__studioFixture.releaseStatus)).toBe("function");
  await page.getByRole("button", { name: "Favorite Slow drift · 42", exact: true }).click();
  await expect(page.getByRole("button", { name: "Unfavorite Slow drift · 42" })).toBeVisible();
  await page.evaluate(() => (window as any).__studioFixture.releaseStatus());
  await page.waitForTimeout(100);
  await expect(page.getByRole("button", { name: "Unfavorite Slow drift · 42" })).toHaveAttribute("aria-pressed", "true");
});

test("locking clinician creation clears an open study recipe", async ({ page }) => {
  await desktopFixture(page, true);
  await page.evaluate(() => {
    const state = (window as any).__studioFixture.state;
    state.can_generate = true;
    state.clinician_unlocked = true;
    state.music_preferences = { provider: "ace_step", style: "focus", duration_s: 60, tempo_bpm: 76, prompt: "A clinician's draft" };
  });
  await expect(page.getByLabel("Describe your sound")).toHaveValue("A clinician's draft");
  await page.evaluate(() => {
    const state = (window as any).__studioFixture.state;
    state.can_generate = false;
    state.clinician_unlocked = false;
    state.music_preferences = { provider: "composer", style: "drift", duration_s: 60, tempo_bpm: 54, prompt: "" };
  });
  await expect(page.getByLabel("Create with")).toHaveValue("composer");
  await expect(page.getByLabel("Describe your sound")).toHaveCount(0);
});

test("exports the selected track and allows retry after export failure", async ({ page }) => {
  await desktopFixture(page);
  const button = page.getByRole("button", { name: "Save Slow drift · 42 as WAV" });
  await button.click();
  await expect(page.getByText(/Saved a checked listening copy/)).toBeVisible();
  expect(await page.evaluate(() => (window as any).__studioFixture.calls.find((c: any) => c.cmd === "library_export").args))
    .toEqual({ participantId: "self", trackId: "track-42", path: "/fixture/export/slow-drift_listening.wav" });
  await page.evaluate(() => { (window as any).__studioFixture.exportError = "Your listening settings changed. Export again using the new settings."; });
  await button.click();
  await expect(page.getByRole("alert")).toContainText("Your listening settings changed");
  await expect(button).toBeEnabled();
});

test("cancelling the save dialog does not export or start playback", async ({ page }) => {
  await desktopFixture(page);
  await page.evaluate(() => { (window as any).__studioFixture.savePath = null; });
  await page.getByRole("button", { name: "Save Slow drift · 42 as WAV" }).click();
  expect(await page.evaluate(() => (window as any).__studioFixture.calls.some((c: any) => ["library_export", "player_play"].includes(c.cmd)))).toBe(false);
  await expect(page.getByRole("button", { name: "Save Slow drift · 42 as WAV" })).toBeEnabled();
});

test("desktop and mobile layouts fit without overflow in both themes", async ({ page }, testInfo) => {
  await desktopFixture(page);
  await page.screenshot({ path: testInfo.outputPath("studio-desktop.png"), fullPage: true });
  for (const colorScheme of ["light", "dark"] as const) {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.emulateMedia({ colorScheme, reducedMotion: "reduce" });
    await expect(page.getByRole("button", { name: "Compose my music" })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
    await page.screenshot({ path: testInfo.outputPath(`studio-mobile-${colorScheme}.png`), fullPage: true });
  }
});
