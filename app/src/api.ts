import { invoke } from "@tauri-apps/api/core";

export type PlayState = "idle" | "loading" | "playing" | "paused";
export type Answer = "yes" | "no" | "louder" | "softer" | "equal" | "first" | "second" | "same";
export type Ear = "both" | "left" | "right";
export type TinnitusSound = "tonal" | "noise_like";
export type Arm = "notch" | "passthrough" | "sham";
export type MusicStyle = "drift" | "focus" | "unwind";
export type MusicProvider = "composer" | "ace_step";
export interface GenerationRequest {
  style: MusicStyle;
  duration_s: number;
  tempo_bpm: number;
  seed: number;
  prompt: string;
}
export interface MusicPreferences extends Omit<GenerationRequest, "seed"> {
  provider: MusicProvider;
}
export interface MusicRecipe { provider: MusicProvider; request: GenerationRequest }
export interface TrackFeedbackRequest {
  participant_id: string;
  track_id: string;
  rating: number;
  discomfort: boolean | null;
}
export interface LibraryTrack {
  id: string;
  title: string;
  duration_s: number;
  favorite: boolean;
  recipe: MusicRecipe | null;
}
export interface GenerationJob {
  stage: string;
  title: string | null;
  track_id: string | null;
  message: string | null;
}

export interface DayTotal {
  date: string;
  minutes: number;
}

export interface Status {
  setup_done: boolean;
  participant_id: string | null;
  daily_goal_min: number;
  today_min: number;
  week: DayTotal[];
  player: { state: PlayState; can_rate: boolean; track_id: string | null; title: string | null; position_s: number; duration_s: number };
  volume_db: number;
  library: LibraryTrack[];
  music_preferences: MusicPreferences;
  library_dir: string;
  clinician_unlocked: boolean;
  pin_set: boolean;
  device_fs: number | null;
  matching_active: boolean;
  error: string | null;
  generation: GenerationJob;
  can_generate: boolean;
}

export interface MatchSummary {
  pitch_hz: number;
  pitch_matches_hz: number[];
  pitch_spread_oct: number;
  octave_shift: number;
  loudness_sl_db: number | null;
  mml_sl_db: number | null;
  retest_recommended: boolean;
  warnings: string[];
  trials: number;
  report_file: string;
  self_setup: boolean;
}

export type MatchView =
  | {
      kind: "prompt";
      stage: string;
      step: number;
      steps: number;
      question: string;
      options: { answer: Answer; label: string }[];
      trial: number;
    }
  | { kind: "done"; summary: MatchSummary };

export interface Profile {
  participant_id: string;
  arm: Arm;
  pitch_hz: number | null;
  sham_hz: number | null;
  width_oct: number;
  daily_goal_min: number;
  matching_report: string | null;
  updated_at: string;
}

export interface VerifyRow {
  title: string;
  file: string;
  duration_s: number;
  all_pass: boolean;
  attenuation_db: number | null;
  input_lufs: number;
  failed_checks: string[];
  warnings: string[];
}

export interface LogEntry {
  participant_id: string;
  track: string;
  started_at: string;
  ended_at: string;
  seconds: number;
  volume_db: number;
  report: string;
}

export const api = {
  status: () => invoke<Status>("status"),
  playTrack: (trackId: string) => invoke<void>("player_play", { trackId }),
  saveMusicPreferences: (participantId: string, preferences: MusicPreferences) => invoke<void>("music_preferences_save", { participantId, preferences }),
  favorite: (participantId: string, trackId: string, favorite: boolean) => invoke<void>("library_favorite", { participantId, trackId, favorite }),
  saveTrackFeedback: (feedback: TrackFeedbackRequest) => invoke<void>("music_feedback_save", { feedback }),
  exportTrack: (participantId: string, trackId: string, path: string) => invoke<{ path: string; report_file: string }>("library_export", { participantId, trackId, path }),
  toggle: () => invoke<void>("player_toggle"),
  skip: (delta: number) => invoke<void>("player_skip", { delta }),
  stop: () => invoke<void>("player_stop"),
  setVolume: (db: number) => invoke<void>("set_volume", { db }),
  matchStart: (opts: { ear: Ear; sound: TinnitusSound; repeats: number }) => invoke<MatchView>("match_start", { opts }),
  matchPlay: () => invoke<void>("match_play"),
  matchAnswer: (answer: Answer) => invoke<MatchView>("match_answer", { answer }),
  matchCancel: () => invoke<void>("match_cancel"),
  unlock: (pin: string) => invoke<boolean>("clinician_unlock", { pin }),
  setPin: (pin: string) => invoke<void>("clinician_set_pin", { pin }),
  lock: () => invoke<void>("clinician_lock"),
  profileGet: () => invoke<Profile | null>("profile_get"),
  profileSave: (profile: Profile) => invoke<void>("profile_save", { profile }),
  lastMatchPitch: () => invoke<[number, string] | null>("last_match_pitch"),
  librarySetDir: (dir: string) => invoke<void>("library_set_dir", { dir }),
  libraryVerify: () => invoke<VerifyRow[]>("library_verify"),
  logsGet: () => invoke<{ entries: LogEntry[]; days: DayTotal[] }>("logs_get"),
  logsExport: (path: string) => invoke<number>("logs_export", { path }),
  dataDir: () => invoke<string>("data_dir"),
  generate: (provider: MusicProvider, request: GenerationRequest) => invoke<void>("generation_start", { provider, request }),
  generationCancel: () => invoke<void>("generation_cancel"),
  generationHealth: () => invoke<void>("generation_health"),
};

export function errorText(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : JSON.stringify(e);
}

export function fmtTime(s: number): string {
  const m = Math.floor(s / 60);
  return `${m}:${String(Math.floor(s % 60)).padStart(2, "0")}`;
}
