import { demoRuntime } from '../lod/main.ts';
import { installVisualDiagnosticProtocol } from '../../engine/diagnostics/visual-protocol.ts';
installVisualDiagnosticProtocol(demoRuntime);
