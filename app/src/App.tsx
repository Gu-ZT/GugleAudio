import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import './styles.css';

type TransportKind = 'local' | 'virtual' | 'network';
type NodeDirection = 'input' | 'output';

type RouteNode = {
  id: string;
  name: string;
  transport: TransportKind;
  direction: NodeDirection;
};

type RouteGraph = {
  nodes: RouteNode[];
  edges: { sourceId: string; targetId: string }[];
};

const CONNECTION_COLORS = [
  '#a78bfa', // purple
  '#f97316', // orange
  '#06b6d4', // cyan
  '#f43f5e', // rose
  '#22c55e', // green
  '#eab308', // yellow
  '#3b82f6', // blue
  '#ec4899', // pink
];

function getConnectionColor(index: number): string {
  return CONNECTION_COLORS[index % CONNECTION_COLORS.length];
}

function App() {
  const [graph, setGraph] = useState<RouteGraph | null>(null);
  const [dragging, setDragging] = useState<{
    sourceId: string;
    startX: number;
    startY: number;
    currentX: number;
    currentY: number;
  } | null>(null);
  const [statusMessage, setStatusMessage] = useState('');
  const [, setRenderTick] = useState(0);
  const containerRef = useRef<HTMLDivElement>(null);

  const loadGraph = useCallback(async () => {
    const g = await invoke<RouteGraph>('get_route_graph');
    setGraph(g);
  }, []);

  useEffect(() => { loadGraph(); }, [loadGraph]);

  // Force re-render after layout so SVG positions are correct
  useLayoutEffect(() => {
    const id = requestAnimationFrame(() => setRenderTick((t) => t + 1));
    return () => cancelAnimationFrame(id);
  }, [graph]);

  const outputs = graph?.nodes.filter((n) => n.direction === 'output') ?? [];
  const inputs = graph?.nodes.filter((n) => n.direction === 'input') ?? [];

  // --- Connection point positions ---
  const getOutputDotId = (nodeId: string) => `dot-out-${nodeId}`;
  const getInputDotId = (nodeId: string) => `dot-in-${nodeId}`;

  const getDotCenter = (dotId: string): { x: number; y: number } | null => {
    const el = document.getElementById(dotId);
    const container = containerRef.current;
    if (!el || !container) return null;
    const elRect = el.getBoundingClientRect();
    const containerRect = container.getBoundingClientRect();
    return {
      x: elRect.left + elRect.width / 2 - containerRect.left,
      y: elRect.top + elRect.height / 2 - containerRect.top,
    };
  };

  const isNetworkNode = (id: string) => {
    return graph?.nodes.find((n) => n.id === id)?.transport === 'network';
  };

  // --- Drag to connect ---
  const onDotMouseDown = (e: React.MouseEvent, sourceId: string) => {
    e.preventDefault();
    const container = containerRef.current;
    if (!container) return;
    const rect = container.getBoundingClientRect();
    const dot = getDotCenter(getOutputDotId(sourceId));
    if (!dot) return;
    setDragging({
      sourceId,
      startX: dot.x,
      startY: dot.y,
      currentX: e.clientX - rect.left,
      currentY: e.clientY - rect.top,
    });
  };

  const onMouseMove = (e: React.MouseEvent) => {
    if (!dragging || !containerRef.current) return;
    const rect = containerRef.current.getBoundingClientRect();
    setDragging({
      ...dragging,
      currentX: e.clientX - rect.left,
      currentY: e.clientY - rect.top,
    });
  };

  const onMouseUp = async (e: React.MouseEvent) => {
    if (!dragging) return;
    const target = (e.target as HTMLElement).closest('[data-input-id]');
    const targetId = target?.getAttribute('data-input-id');

    if (targetId) {
      if (isNetworkNode(dragging.sourceId) && isNetworkNode(targetId)) {
        setStatusMessage('Network nodes cannot connect to each other.');
      } else {
        try {
          const g = await invoke<RouteGraph>('add_route', {
            edge: { sourceId: dragging.sourceId, targetId },
          });
          setGraph(g);
          setStatusMessage('');
        } catch (err) {
          setStatusMessage(String(err));
        }
      }
    }
    setDragging(null);
  };

  const removeEdge = async (sourceId: string, targetId: string) => {
    const g = await invoke<RouteGraph>('remove_route', { sourceId, targetId });
    setGraph(g);
  };

  const refreshDevices = async () => {
    const g = await invoke<RouteGraph>('refresh_audio_devices');
    setGraph(g);
    setStatusMessage('Devices refreshed.');
    setTimeout(() => setStatusMessage(''), 2000);
  };

  // --- Render bezier connections ---
  const renderConnections = () => {
    if (!graph) return null;
    const paths: React.ReactNode[] = [];

    graph.edges.forEach((edge, i) => {
      const start = getDotCenter(getOutputDotId(edge.sourceId));
      const end = getDotCenter(getInputDotId(edge.targetId));
      if (!start || !end) return;

      const color = getConnectionColor(i);
      const cpOffset = Math.abs(end.x - start.x) * 0.4;
      const d = `M ${start.x} ${start.y} C ${start.x + cpOffset} ${start.y}, ${end.x - cpOffset} ${end.y}, ${end.x} ${end.y}`;

      paths.push(
        <g key={`edge-${i}`} onClick={() => removeEdge(edge.sourceId, edge.targetId)} style={{ cursor: 'pointer' }}>
          <path d={d} stroke={color} strokeWidth={3} fill="none" opacity={0.9} />
          <circle cx={start.x} cy={start.y} r={6} fill={color} />
          <circle cx={end.x} cy={end.y} r={6} fill={color} />
        </g>,
      );
    });

    // Dragging preview
    if (dragging) {
      const cpOffset = Math.abs(dragging.currentX - dragging.startX) * 0.4;
      const d = `M ${dragging.startX} ${dragging.startY} C ${dragging.startX + cpOffset} ${dragging.startY}, ${dragging.currentX - cpOffset} ${dragging.currentY}, ${dragging.currentX} ${dragging.currentY}`;
      paths.push(
        <path key="drag-preview" d={d} stroke="#94a3b8" strokeWidth={2} fill="none" strokeDasharray="6 4" />,
      );
    }

    return paths;
  };

  const transportIcon = (transport: TransportKind) => {
    switch (transport) {
      case 'local': return '🎧';
      case 'virtual': return '🔀';
      case 'network': return '🌐';
    }
  };

  return (
    <div
      className="app-container"
      ref={containerRef}
      onMouseMove={onMouseMove}
      onMouseUp={onMouseUp}
    >
      {/* Header */}
      <header className="app-header">
        <h1 className="app-title">GugleAudio</h1>
        <div className="header-actions">
          <button className="btn-refresh" onClick={refreshDevices}>Refresh</button>
        </div>
      </header>

      {statusMessage && <div className="status-bar">{statusMessage}</div>}

      {/* Main content */}
      <div className="panels-container">
        {/* Left panel - Outputs (sources) */}
        <div className="panel panel-left">
          <div className="panel-header">
            <span>Inputs (Sources)</span>
          </div>
          <div className="device-list">
            {outputs.map((node) => (
              <div key={node.id} className="device-card">
                <div className="device-info">
                  <span className="device-icon">{transportIcon(node.transport)}</span>
                  <div className="device-text">
                    <div className="device-name">{node.name}</div>
                    <div className="device-sub">{node.transport}</div>
                  </div>
                </div>
                <div className="device-controls">
                  <div className="volume-bar" />
                  <div
                    className="connection-dot dot-output"
                    id={getOutputDotId(node.id)}
                    onMouseDown={(e) => onDotMouseDown(e, node.id)}
                  />
                </div>
              </div>
            ))}
          </div>
        </div>

        {/* SVG overlay for connections */}
        <svg className="connections-svg">
          {renderConnections()}
        </svg>

        {/* Right panel - Inputs (sinks) */}
        <div className="panel panel-right">
          <div className="panel-header">
            <span>Outputs (Sinks)</span>
          </div>
          <div className="device-list">
            {inputs.map((node) => (
              <div key={node.id} className="device-card" data-input-id={node.id}>
                <div className="device-controls-left">
                  <div
                    className="connection-dot dot-input"
                    id={getInputDotId(node.id)}
                    data-input-id={node.id}
                  />
                  <div className="volume-bar" />
                </div>
                <div className="device-info">
                  <span className="device-icon">{transportIcon(node.transport)}</span>
                  <div className="device-text">
                    <div className="device-name">{node.name}</div>
                    <div className="device-sub">{node.transport}</div>
                  </div>
                </div>
              </div>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}

export default App;
