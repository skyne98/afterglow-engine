#!/usr/bin/env bun
/**
 * CLI migration runner.
 *
 *   bun run migrate   # create/open the DB and apply pending migrations
 *
 * Exit code is non-zero when a migration fails. The runner is idempotent:
 * re-running it applies nothing new.
 */
import { openDb, MIGRATIONS_DIR } from "./db";

const db = openDb({}, MIGRATIONS_DIR);
const ran = db.query<{ name: string; applied_at: string }, []>(
  `SELECT name, applied_at FROM _schema_migrations ORDER BY name`,
).all();

console.log(`DB ready. Applied migrations (${ran.length}):`);
for (const r of ran) console.log(`  - ${r.name} (${r.applied_at})`);
db.close();
