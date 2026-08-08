import { demoRuntime } from '../dungeon/main.ts';
import { installVisualDiagnosticProtocol } from '../../engine/diagnostics/visual-protocol.ts';
installVisualDiagnosticProtocol(demoRuntime);
