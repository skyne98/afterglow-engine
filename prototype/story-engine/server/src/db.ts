import { Database } from "bun:sqlite";
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { applyMigrations } from "./migrations";

/** Default database file location relative to the server source root. */
const DEFAULT_DB_PATH = join(import.meta.dir, "..", "data", "story.db");

export interface DbConfig {
  path?: string;
  /** True to create the file when it does not exist (Bun default). */
  create?: boolean;
}

export function openDb(config: DbConfig = {}, migrationsDir?: string): Database {
  const path = config.path ?? process.env.STORY_DB_PATH ?? DEFAULT_DB_PATH;
  if (config.create ?? true) {
    mkdirSync(dirname(path), { recursive: true });
  }
  const db = new Database(path, { create: config.create ?? true });
  db.exec("PRAGMA journal_mode = WAL");
  db.exec("PRAGMA foreign_keys = ON");
  if (migrationsDir) {
    applyMigrations(db, migrationsDir);
  }
  return db;
}

/** Migrations directory resolved from this module's location. */
export const MIGRATIONS_DIR = join(import.meta.dir, "..", "migrations");
