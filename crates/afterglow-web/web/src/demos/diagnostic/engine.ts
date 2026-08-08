import { demoRuntime } from '../engine/main.ts';
import { installVisualDiagnosticProtocol } from '../../engine/diagnostics/visual-protocol.ts';
installVisualDiagnosticProtocol(demoRuntime);
