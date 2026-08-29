import { Database } from "bun:sqlite";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

/**
 * Minimal migration runner.
 *
 * SQL files live in `server/migrations/` and are named NN_name.sql. Each file
 * is a single transaction. A `_schema_migrations` table records the applied
 * names so older environments never re-run a migration. Rows are keyed by
 * file name, not number, so renames are explicit.
 */

export interface Migration {
  name: string;
  sql: string;
}

export function loadMigrations(dir: string): Migration[] {
  return readdirSync(dir)
    .filter((f) => f.endsWith(".sql"))
    .sort()
    .map((f) => ({ name: f, sql: readFileSync(join(dir, f), "utf8") }));
}

export function appliedMigrations(db: Database): Set<string> {
  db.run(`CREATE TABLE IF NOT EXISTS _schema_migrations (
    name TEXT PRIMARY KEY,
    applied_at TEXT NOT NULL
  )`);
  const rows = db
    .query<{ name: string }, []>(`SELECT name FROM _schema_migrations`)
    .all();
  return new Set(rows.map((r) => r.name));
}

export function applyMigrations(db: Database, dir: string): string[] {
  const applied = appliedMigrations(db);
  const ran: string[] = [];
  db.exec("BEGIN");
  try {
    for (const m of loadMigrations(dir)) {
      if (applied.has(m.name)) continue;
      db.exec(m.sql);
      db.run(`INSERT INTO _schema_migrations (name, applied_at) VALUES (?, ?)`, [
        m.name,
        new Date().toISOString(),
      ]);
      ran.push(m.name);
    }
    db.exec("COMMIT");
  } catch (err) {
    db.exec("ROLLBACK");
    throw err;
  }
  return ran;
}
