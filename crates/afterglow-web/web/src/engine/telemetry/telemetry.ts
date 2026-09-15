import { defineResource, type Resource } from '../core/resource.ts';
import { Telemetry } from '../../../../../afterglow-telemetry/web/src/telemetry.ts';

export * from '../../../../../afterglow-telemetry/web/src/telemetry.ts';
export { Telemetry as EngineTelemetry };

/** The engine adapter registers the generic producer as an ECS resource. */
export const TelemetryRes: Resource<Telemetry> = defineResource<Telemetry>('telemetry', () => {
  throw new Error('Telemetry not initialized. Set TelemetryRes during bootstrap.');
});
