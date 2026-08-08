import { demoRuntime } from '../rigged-vt/main.ts';
import { installVisualDiagnosticProtocol } from '../../engine/diagnostics/visual-protocol.ts';
installVisualDiagnosticProtocol(demoRuntime);
