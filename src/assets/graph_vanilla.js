// NuAnalytics curriculum graph — vanilla JS renderer
// Supports multiple independent graph instances on a single page.
// Each graph is identified by a graphId string used as a DOM prefix.
//
// API (called by the Rust renderer's inline <script> blocks):
//   nuGraphs.register(graphId, { edges, criticalPath, nodes })
//   nuGraphs.draw(graphId)
//
// Idempotent: safe to include multiple times on a page.

if (!window.nuGraphs) {
    window.nuGraphs = (function () {
        'use strict';

        // ── Registry ───────────────────────────────────────────────────────
        // graphId -> { edges, criticalPath, nodes, nodeIndex, adjacency }
        var _registry = {};
        // Currently open modal element (null when no modal is open).
        var _openModal = null;

        // ── Public: register graph data ────────────────────────────────────
        function register(graphId, data) {
            var nodes = data.nodes || [];
            // Build a courseId -> node index for O(1) lookup in the modal.
            var nodeIndex = {};
            nodes.forEach(function (n) { nodeIndex[n.id] = n; });
            _registry[graphId] = {
                edges: data.edges || [],
                criticalPath: data.criticalPath || [],
                nodes: nodes,
                nodeIndex: nodeIndex,
                adjacency: null  // built lazily on first draw/hover
            };
        }

        // ── Build adjacency maps for hover path tracing ────────────────────
        function buildAdjacency(graphId) {
            var g = _registry[graphId];
            if (!g || g.adjacency) return;

            var forwardEdges  = new Map(); // course -> [courses it leads to]
            var backwardEdges = new Map(); // course -> [courses that lead to it]
            var corequisites  = new Map(); // course -> Set of corequisites

            g.edges.forEach(function (edge) {
                if (!forwardEdges.has(edge.from))  forwardEdges.set(edge.from, []);
                if (!backwardEdges.has(edge.to))   backwardEdges.set(edge.to, []);
                forwardEdges.get(edge.from).push(edge.to);
                backwardEdges.get(edge.to).push(edge.from);

                if (edge.dashes) {
                    if (!corequisites.has(edge.from)) corequisites.set(edge.from, new Set());
                    if (!corequisites.has(edge.to))   corequisites.set(edge.to,   new Set());
                    corequisites.get(edge.from).add(edge.to);
                    corequisites.get(edge.to).add(edge.from);
                }
            });

            g.adjacency = {
                forwardEdges:  forwardEdges,
                backwardEdges: backwardEdges,
                corequisites:  corequisites,
                criticalSet:   new Set(g.criticalPath)
            };
        }

        // ── Trace chains through a course ──────────────────────────────────
        function getChainsThrough(graphId, courseId) {
            buildAdjacency(graphId);
            var adj = _registry[graphId] && _registry[graphId].adjacency;
            if (!adj) return { courses: new Set([courseId]), edges: new Set() };

            var inChain     = new Set([courseId]);
            var edgesInChain = new Set();

            function traceBackward(id) {
                var prev = adj.backwardEdges.get(id) || [];
                prev.forEach(function (prevId) {
                    edgesInChain.add(prevId + '->' + id);
                    if (!inChain.has(prevId)) {
                        inChain.add(prevId);
                        traceBackward(prevId);
                    }
                });
            }

            function traceForward(id) {
                var next = adj.forwardEdges.get(id) || [];
                next.forEach(function (nextId) {
                    edgesInChain.add(id + '->' + nextId);
                    if (!inChain.has(nextId)) {
                        inChain.add(nextId);
                        traceForward(nextId);
                    }
                });
            }

            traceBackward(courseId);
            traceForward(courseId);

            // Include corequisites of every course in the chain.
            Array.from(inChain).forEach(function (course) {
                var coreqs = adj.corequisites.get(course);
                if (coreqs) {
                    coreqs.forEach(function (coreq) {
                        inChain.add(coreq);
                        edgesInChain.add(course + '->' + coreq);
                        edgesInChain.add(coreq  + '->' + course);
                    });
                }
            });

            return { courses: inChain, edges: edgesInChain };
        }

        // ── Draw SVG connections for one graph ─────────────────────────────
        function draw(graphId) {
            var g = _registry[graphId];
            if (!g) return;

            var graphEl   = document.getElementById('graph-' + graphId);
            var svgEl     = document.getElementById('svg-'   + graphId);
            if (!graphEl || !svgEl) return;

            var wrapper     = svgEl.parentElement;
            var wrapperRect = wrapper.getBoundingClientRect();

            svgEl.style.width  = wrapper.offsetWidth  + 'px';
            svgEl.style.height = wrapper.offsetHeight + 'px';
            svgEl.setAttribute('width',  wrapper.offsetWidth);
            svgEl.setAttribute('height', wrapper.offsetHeight);
            svgEl.innerHTML = '';

            var svgNS = 'http://www.w3.org/2000/svg';

            g.edges.forEach(function (edge) {
                var fromEl = graphEl.querySelector('[data-course-id="' + edge.from + '"]');
                var toEl   = graphEl.querySelector('[data-course-id="' + edge.to   + '"]');
                if (!fromEl || !toEl) return;

                var fromRect = fromEl.getBoundingClientRect();
                var toRect   = toEl.getBoundingClientRect();
                var offsetX  = wrapperRect.left;
                var offsetY  = wrapperRect.top;

                var fromCX = fromRect.left + fromRect.width  / 2;
                var toCX   = toRect.left   + toRect.width   / 2;
                var sameColumn = Math.abs(fromCX - toCX) < 50;

                var x1, y1, x2, y2, pathD;

                if (sameColumn && edge.dashes) {
                    // Corequisite in the same term: connect bottom of upper to top of lower.
                    var upper = (fromRect.top < toRect.top) ? fromRect : toRect;
                    var lower = (fromRect.top < toRect.top) ? toRect   : fromRect;
                    x1 = upper.left + upper.width / 2 - offsetX;
                    y1 = upper.bottom - offsetY;
                    x2 = lower.left + lower.width / 2 - offsetX;
                    y2 = lower.top   - offsetY;
                    var midY1 = (y1 + y2) / 2;
                    pathD = 'M ' + x1 + ' ' + y1 +
                            ' C ' + x1 + ' ' + midY1 + ', ' + x2 + ' ' + midY1 + ', ' + x2 + ' ' + y2;
                } else if (sameColumn) {
                    // Non-coreq same column (rare).
                    x1 = fromRect.left + fromRect.width  / 2 - offsetX;
                    y1 = fromRect.bottom - offsetY;
                    x2 = toRect.left   + toRect.width   / 2 - offsetX;
                    y2 = toRect.top    - offsetY;
                    var midY2 = (y1 + y2) / 2;
                    pathD = 'M ' + x1 + ' ' + y1 +
                            ' C ' + x1 + ' ' + midY2 + ', ' + x2 + ' ' + midY2 + ', ' + x2 + ' ' + y2;
                } else {
                    // Cross-term: right edge to left edge.
                    x1 = fromRect.right - offsetX;
                    y1 = fromRect.top + fromRect.height / 2 - offsetY;
                    x2 = toRect.left  - offsetX;
                    y2 = toRect.top   + toRect.height  / 2 - offsetY;
                    var midX = (x1 + x2) / 2;
                    pathD = 'M ' + x1 + ' ' + y1 +
                            ' C ' + midX + ' ' + y1 + ', ' + midX + ' ' + y2 + ', ' + x2 + ' ' + y2;
                }

                var path = document.createElementNS(svgNS, 'path');
                path.setAttribute('d', pathD);
                path.dataset.from = edge.from;
                path.dataset.to   = edge.to;
                path.setAttribute('class', edge.dashes ? 'coreq-line' : 'prereq-line');
                svgEl.appendChild(path);
            });
        }

        // ── Redraw all registered graphs ───────────────────────────────────
        function drawAll() {
            Object.keys(_registry).forEach(draw);
        }

        // ── Hover handlers ─────────────────────────────────────────────────
        function handleHover(e) {
            var node      = e.currentTarget;
            var courseId  = node.dataset.courseId;
            var graphId   = node.dataset.graphId;
            if (!courseId || !graphId) return;

            buildAdjacency(graphId);
            var adj = _registry[graphId] && _registry[graphId].adjacency;
            if (!adj) return;

            var graphEl  = document.getElementById('graph-' + graphId);
            var svgEl    = document.getElementById('svg-'   + graphId);
            if (!graphEl || !svgEl) return;

            var chain    = getChainsThrough(graphId, courseId);
            var onCrit   = adj.criticalSet.has(courseId);

            graphEl.querySelectorAll('.course-node').forEach(function (n) {
                var id = n.dataset.courseId;
                if (chain.courses.has(id)) {
                    n.classList.remove('faded');
                    if (onCrit && adj.criticalSet.has(id)) {
                        n.classList.add('critical-highlight');
                    } else {
                        n.classList.add('highlighted');
                    }
                } else {
                    n.classList.add('faded');
                    n.classList.remove('highlighted', 'critical-highlight');
                }
            });

            svgEl.querySelectorAll('path').forEach(function (path) {
                var key = path.dataset.from + '->' + path.dataset.to;
                if (chain.edges.has(key)) {
                    path.classList.remove('faded');
                    if (onCrit && adj.criticalSet.has(path.dataset.from) && adj.criticalSet.has(path.dataset.to)) {
                        path.classList.add('critical');
                    } else {
                        path.classList.add('highlighted');
                    }
                } else {
                    path.classList.add('faded');
                    path.classList.remove('highlighted', 'critical');
                }
            });
        }

        function handleLeave(e) {
            var graphId = e.currentTarget.dataset.graphId;
            if (!graphId) return;
            var graphEl = document.getElementById('graph-' + graphId);
            var svgEl   = document.getElementById('svg-'   + graphId);
            if (!graphEl || !svgEl) return;
            graphEl.querySelectorAll('.course-node').forEach(function (n) {
                n.classList.remove('faded', 'highlighted', 'critical-highlight');
            });
            svgEl.querySelectorAll('path').forEach(function (p) {
                p.classList.remove('faded', 'highlighted', 'critical');
            });
        }

        // ── Attach hover listeners to all course nodes ─────────────────────
        function attachHoverHandlers() {
            document.querySelectorAll('.nu-graph .course-node').forEach(function (node) {
                node.removeEventListener('mouseenter', handleHover);
                node.removeEventListener('mouseleave', handleLeave);
                node.addEventListener('mouseenter', handleHover);
                node.addEventListener('mouseleave', handleLeave);
            });
        }

        // ── Course detail modal ────────────────────────────────────────────
        function handleClick(e) {
            var node     = e.currentTarget;
            var graphId  = node.dataset.graphId;
            var courseId = node.dataset.courseId;
            if (graphId && courseId) openModal(graphId, courseId);
        }

        function attachClickHandlers() {
            document.querySelectorAll('.nu-graph .course-node').forEach(function (node) {
                node.removeEventListener('click', handleClick);
                node.addEventListener('click', handleClick);
            });
        }

        // Render one row of the metrics table.
        function metricRow(label, value, median) {
            var fmt = function (v) {
                if (v === null || v === undefined) return '—';
                // Integers render plain; floats render with one decimal.
                return (Math.floor(v) === v) ? String(v) : v.toFixed(1);
            };
            return '<tr><td>' + label + '</td>' +
                   '<td>' + fmt(value)  + '</td>' +
                   '<td>' + fmt(median) + '</td></tr>';
        }

        function openModal(graphId, courseId) {
            var g = _registry[graphId];
            if (!g) return;
            var n = g.nodeIndex[courseId];
            if (!n) return;

            closeModal();  // ensure only one modal is open at a time

            var backdrop = document.createElement('div');
            backdrop.className = 'nu-graph-modal-backdrop';
            backdrop.setAttribute('role', 'dialog');
            backdrop.setAttribute('aria-modal', 'true');

            // Build inner card
            var card = document.createElement('div');
            card.className = 'nu-graph-modal';

            var subtitle = (n.credits != null) ? (n.credits + ' credits') : '';

            card.innerHTML =
                '<button class="nu-graph-modal-close" aria-label="Close">×</button>' +
                '<h3 class="nu-graph-modal-title">' + escapeHtml(n.id) +
                ' — ' + escapeHtml(n.name || '') + '</h3>' +
                '<p class="nu-graph-modal-subtitle">' + escapeHtml(subtitle) + '</p>' +
                '<table class="nu-graph-modal-table">' +
                '<thead><tr><th>Metric</th><th>This plan</th><th>Median</th></tr></thead>' +
                '<tbody>' +
                metricRow('Complexity', n.complexity, n.medianComplexity) +
                metricRow('Delay',      n.delay,      n.medianDelay) +
                metricRow('Blocking',   n.blocking,   n.medianBlocking) +
                '</tbody></table>';

            backdrop.appendChild(card);

            // Close handlers
            backdrop.addEventListener('click', function (e) {
                if (e.target === backdrop) closeModal();
            });
            card.querySelector('.nu-graph-modal-close')
                .addEventListener('click', closeModal);

            document.body.appendChild(backdrop);
            _openModal = backdrop;
        }

        function closeModal() {
            if (!_openModal) return;
            if (_openModal.parentNode) _openModal.parentNode.removeChild(_openModal);
            _openModal = null;
        }

        // Plain HTML escape — modal content is built from registry strings,
        // not user input, but defensive escaping is cheap.
        function escapeHtml(s) {
            return String(s)
                .replace(/&/g, '&amp;')
                .replace(/</g, '&lt;')
                .replace(/>/g, '&gt;')
                .replace(/"/g, '&quot;');
        }

        // ── Initialise on DOMContentLoaded ─────────────────────────────────
        document.addEventListener('DOMContentLoaded', function () {
            requestAnimationFrame(function () {
                requestAnimationFrame(function () {
                    drawAll();
                    attachHoverHandlers();
                    attachClickHandlers();
                    // Extra passes for headless Chrome / PDF generation.
                    setTimeout(drawAll, 200);
                    setTimeout(drawAll, 500);
                    setTimeout(drawAll, 1000);
                });
            });
            window.addEventListener('resize', drawAll);
            document.addEventListener('keydown', function (e) {
                if (e.key === 'Escape') closeModal();
            });
        });

        // ── Print / PDF support ────────────────────────────────────────────
        window.addEventListener('beforeprint', drawAll);
        if (window.matchMedia) {
            var mq = window.matchMedia('print');
            if (mq.matches) {
                drawAll();
                setTimeout(drawAll, 100);
            }
            if (mq.addEventListener) {
                mq.addEventListener('change', function (e) {
                    if (e.matches) { drawAll(); setTimeout(drawAll, 100); }
                });
            }
        }

        return {
            register:            register,
            draw:                draw,
            drawAll:             drawAll,
            attachHoverHandlers: attachHoverHandlers,
            attachClickHandlers: attachClickHandlers,
            openModal:           openModal,
            closeModal:          closeModal
        };
    }());
}
