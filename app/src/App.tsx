import { useEffect, useMemo, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';

type TransportKind = 'local' | 'virtual' | 'network';
type NodeDirection = 'input' | 'output';
type EngineState = 'stopped' | 'starting' | 'running';

type RouteNode = {
  id: string;
  name: string;
  transport: TransportKind;
  direction: NodeDirection;
};

type RouteGraph = {
  nodes: RouteNode[];
  edges: { source_id: string; target_id: string }[];
};

type EngineSnapshot = {
  state: EngineState;
  active_session: string | null;
  processed_frames: number;
};

type ValidationError = {
  code: string;
  detail?: string;
};

const panelStyle: React.CSSProperties = {
  background: '#171a22',
  border: '1px solid #2a3040',
  borderRadius: 14,
  padding: 20,
};

function App() {
  const [graph, setGraph] = useState<RouteGraph | null>(null);
  const [engine, setEngine] = useState<EngineSnapshot | null>(null);
  const [sourceId, setSourceId] = useState('');
  const [targetId, setTargetId] = useState('');
  const [validationMessage, setValidationMessage] = useState('');
  const [validationError, setValidationError] = useState<ValidationError | null>(null);

  useEffect(() => {
    const load = async () => {
      const [routeGraph, snapshot] = await Promise.all([
        invoke<RouteGraph>('get_route_graph'),
        invoke<EngineSnapshot>('get_engine_snapshot'),
      ]);
      setGraph(routeGraph);
      setEngine(snapshot);
      if (routeGraph.nodes.length > 1) {
        const output = routeGraph.nodes.find((node) => node.direction === 'output');
        const input = routeGraph.nodes.find((node) => node.direction === 'input');
        setSourceId(output?.id ?? '');
        setTargetId(input?.id ?? '');
      }
    };

    load().catch((error) => {
      setValidationMessage(`Failed to load app state: ${String(error)}`);
    });
  }, []);

  const outputs = useMemo(
    () => graph?.nodes.filter((node) => node.direction === 'output') ?? [],
    [graph],
  );
  const inputs = useMemo(
    () => graph?.nodes.filter((node) => node.direction === 'input') ?? [],
    [graph],
  );

  const validateEdge = async () => {
    setValidationMessage('');
    setValidationError(null);

    try {
      await invoke('validate_route_edge', {
        edge: {
          sourceId,
          targetId,
        },
      });
      setValidationMessage('Route is valid.');
    } catch (error) {
      const parsed = error as ValidationError;
      setValidationError(parsed);
      setValidationMessage(parsed.code === 'network_to_network_forbidden'
        ? 'Network nodes cannot connect directly to network nodes.'
        : `Route rejected: ${parsed.code}`);
    }
  };

  const startEngine = async () => {
    const snapshot = await invoke<EngineSnapshot>('start_engine');
    setEngine(snapshot);
  };

  const stopEngine = async () => {
    const snapshot = await invoke<EngineSnapshot>('stop_engine');
    setEngine(snapshot);
  };

  return (
    <main style={{ padding: 24, display: 'grid', gap: 20 }}>
      <header>
        <h1 style={{ margin: 0 }}>GugleAudio</h1>
        <p style={{ color: '#9ba7b4' }}>
          First vertical slice: route graph, network-node constraint, and engine shell.
        </p>
      </header>

      <section style={{ display: 'grid', gridTemplateColumns: '1.1fr 0.9fr', gap: 20 }}>
        <div style={panelStyle}>
          <h2 style={{ marginTop: 0 }}>Nodes</h2>
          <div style={{ display: 'grid', gap: 10 }}>
            {graph?.nodes.map((node) => (
              <div
                key={node.id}
                style={{
                  padding: '10px 12px',
                  borderRadius: 10,
                  background: '#10131a',
                  border: '1px solid #232938',
                }}
              >
                <div>{node.name}</div>
                <small style={{ color: '#94a0ad' }}>
                  {node.transport} · {node.direction} · {node.id}
                </small>
              </div>
            ))}
          </div>
        </div>

        <div style={{ display: 'grid', gap: 20 }}>
          <div style={panelStyle}>
            <h2 style={{ marginTop: 0 }}>Engine</h2>
            <p>Status: <strong>{engine?.state ?? 'loading'}</strong></p>
            <p>Session: <strong>{engine?.active_session ?? 'none'}</strong></p>
            <p>Processed frames: <strong>{engine?.processed_frames ?? 0}</strong></p>
            <div style={{ display: 'flex', gap: 12 }}>
              <button onClick={startEngine}>Start</button>
              <button onClick={stopEngine}>Stop</button>
            </div>
          </div>

          <div style={panelStyle}>
            <h2 style={{ marginTop: 0 }}>Route Validation</h2>
            <div style={{ display: 'grid', gap: 12 }}>
              <label>
                <div style={{ marginBottom: 6 }}>Source node</div>
                <select value={sourceId} onChange={(event) => setSourceId(event.target.value)}>
                  {outputs.map((node) => (
                    <option key={node.id} value={node.id}>
                      {node.name}
                    </option>
                  ))}
                </select>
              </label>

              <label>
                <div style={{ marginBottom: 6 }}>Target node</div>
                <select value={targetId} onChange={(event) => setTargetId(event.target.value)}>
                  {inputs.map((node) => (
                    <option key={node.id} value={node.id}>
                      {node.name}
                    </option>
                  ))}
                </select>
              </label>

              <button onClick={validateEdge}>Validate Route</button>

              {validationMessage && (
                <div
                  style={{
                    borderRadius: 10,
                    padding: 12,
                    background: validationError ? '#3b1720' : '#10281d',
                    border: `1px solid ${validationError ? '#7c2d3f' : '#1d6a3b'}`,
                  }}
                >
                  {validationMessage}
                </div>
              )}
            </div>
          </div>
        </div>
      </section>
    </main>
  );
}

export default App;
