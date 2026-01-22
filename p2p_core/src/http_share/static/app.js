        const els = {
            dropArea: document.getElementById('dropArea'),
            dropIcon: document.getElementById('dropIcon'),
            dropText: document.getElementById('dropText'),
            fileInput: document.getElementById('fileInput'),
            fileName: document.getElementById('fileName'),
            fileSize: document.getElementById('fileSize'),
            browseBtn: document.getElementById('browseBtn'),
            sendBtn: document.getElementById('sendBtn'),
            cancelBtn: document.getElementById('cancelBtn'),
            progressBar: document.getElementById('progressBar'),
            progressText: document.getElementById('progressText'),
            statusText: document.getElementById('statusText'),
            logContainer: document.getElementById('logContainer'),
            toggleLogBtn: document.getElementById('toggleLogBtn')
        };

        let selectedFile = null;
        let ws = null;
        const CHUNK_SIZE = 256 * 1024; // 256KB - optimized for LAN

        // --- Event Listeners ---
        els.browseBtn.addEventListener('click', () => els.fileInput.click());
        els.dropArea.addEventListener('click', () => els.fileInput.click());

        els.fileInput.addEventListener('change', e => {
            if (e.target.files.length) handleFile(e.target.files[0]);
        });

        // Drag & Drop
        ['dragenter', 'dragover', 'dragleave', 'drop'].forEach(eventName => {
            els.dropArea.addEventListener(eventName, preventDefaults, false);
        });

        function preventDefaults(e) {
            e.preventDefault();
            e.stopPropagation();
        }

        ['dragenter', 'dragover'].forEach(eventName => {
            els.dropArea.addEventListener(eventName, () => els.dropArea.style.borderColor = 'var(--accent)', false);
        });

        ['dragleave', 'drop'].forEach(eventName => {
            els.dropArea.addEventListener(eventName, () => els.dropArea.style.borderColor = 'var(--stroke)', false);
        });

        els.dropArea.addEventListener('drop', (e) => {
            const dt = e.dataTransfer;
            const files = dt.files;
            if (files.length) handleFile(files[0]);
        });

        els.sendBtn.addEventListener('click', startUpload);
        els.cancelBtn.addEventListener('click', resetUI);

        // --- Logic ---

        function handleFile(file) {
            selectedFile = file;
            els.fileName.textContent = file.name;
            els.fileName.title = file.name;
            els.fileSize.textContent = formatSize(file.size);

            // Visual feedback on drop zone
            els.dropIcon.className = "ph ph-file";
            els.dropText.textContent = "File Selected";

            els.sendBtn.disabled = false;
            els.cancelBtn.disabled = false;
            updateStatus("Ready to send", "--text-primary");
        }

        function resetUI() {
            if (ws) { ws.close(); ws = null; }
            selectedFile = null;
            els.fileInput.value = '';

            els.fileName.textContent = "None";
            els.fileSize.textContent = "0 B";

            els.dropIcon.className = "ph ph-upload-simple";
            els.dropText.textContent = "Drag and drop a file here";

            els.sendBtn.disabled = true;
            els.cancelBtn.disabled = true;

            els.progressBar.style.width = '0%';
            els.progressText.textContent = '0%';
            updateStatus("Idle", "--text-secondary");
        }

        // --- Logging & Status ---
        els.toggleLogBtn.addEventListener('click', () => {
            const isHidden = els.logContainer.classList.contains('hidden');
            if (isHidden) {
                els.logContainer.classList.remove('hidden');
                els.toggleLogBtn.textContent = 'Hide';
            } else {
                els.logContainer.classList.add('hidden');
                els.toggleLogBtn.textContent = 'Show';
            }
        });

        function log(msg, type = 'info') {
            const div = document.createElement('div');
            const time = new Date().toLocaleTimeString().split(' ')[0]; // HH:MM:SS
            div.textContent = `[${time}] ${msg}`;
            if (type === 'error') div.style.color = 'var(--error)';
            else if (type === 'success') div.style.color = 'var(--success)';
            else if (type === 'warn') div.style.color = 'var(--text-primary)';

            els.logContainer.appendChild(div);
            els.logContainer.scrollTop = els.logContainer.scrollHeight;
        }

        function updateStatus(msg, colorVar) {
            els.statusText.textContent = msg;
            els.statusText.style.color = `var(${colorVar})`;

            let type = 'info';
            if (colorVar.includes('error')) type = 'error';
            else if (colorVar.includes('success')) type = 'success';
            else if (colorVar.includes('accent')) type = 'warn';

            log(msg, type);
        }

        // Capture console errors
        window.onerror = function (message, source, lineno, colno, error) {
            log(`Window Error: ${message}`, 'error');
        };

        function startUpload() {
            if (!selectedFile) return;

            els.sendBtn.disabled = true;
            els.browseBtn.disabled = true; // Lock browsing while sending
            els.dropArea.style.pointerEvents = "none";

            updateStatus("Connecting...", "--text-primary");

            const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
            // Remove trailing slash from pathname if present to avoid double slash
            const path = window.location.pathname.replace(/\/$/, '');
            const wsUrl = `${protocol}//${window.location.host}${path}/ws`;

            log(`Connecting to: ${wsUrl}`);

            try {
                ws = new WebSocket(wsUrl);
                ws.binaryType = 'arraybuffer';
            } catch (err) {
                log(`WebSocket creation failed: ${err}`, 'error');
                updateStatus("Init Error", "--error");
                cleanupAfterTransfer();
                return;
            }

            // Connection timeout watchdog
            const timeoutId = setTimeout(() => {
                if (ws && ws.readyState === WebSocket.CONNECTING) {
                    log("Connection timed out (>5s). Check firewall?", 'error');
                    updateStatus("Timed Out", "--error");
                    ws.close();
                }
            }, 5000);

            ws.onopen = () => {
                clearTimeout(timeoutId);
                log("WebSocket Connected");
                updateStatus("Please approve on PC...", "--text-primary");
                ws.send(JSON.stringify({
                    type: "file_info",
                    file_name: selectedFile.name,
                    file_size: selectedFile.size
                }));
            };

            ws.onmessage = e => handleServerMessage(JSON.parse(e.data));

            ws.onerror = e => {
                clearTimeout(timeoutId);
                log("WebSocket Error event fired", 'error');
                console.error(e);
                updateStatus("Connection Error", "--error");
                cleanupAfterTransfer();
            };

            ws.onclose = e => {
                clearTimeout(timeoutId);
                log(`WebSocket Closed: code=${e.code}, reason=${e.reason}`, 'warn');
                if (!els.sendBtn.disabled && els.progressBar.style.width !== '100%') {
                    updateStatus("Connection Closed", "--error");
                }
                cleanupAfterTransfer();
            };
        }

        function cleanupAfterTransfer() {
            els.browseBtn.disabled = false;
            els.dropArea.style.pointerEvents = "auto";
            if (els.progressBar.style.width !== '100%') {
                els.sendBtn.disabled = false; // Allow retry if not complete
            }
        }

        function handleServerMessage(msg) {
            switch (msg.type) {
                case 'accepted':
                    updateStatus("Uploading...", "--accent");
                    uploadFileChunks();
                    break;
                case 'rejected':
                    updateStatus(`Rejected: ${msg.reason}`, "--error");
                    ws.close();
                    break;
                case 'progress':
                    const p = Math.min(100, (msg.received_bytes / selectedFile.size) * 100);
                    const pStr = p.toFixed(0) + "%";
                    els.progressBar.style.width = pStr;
                    els.progressText.textContent = pStr;
                    break;
                case 'complete':
                    els.progressBar.style.width = '100%';
                    els.progressText.textContent = '100%';
                    updateStatus("Completed", "--success");
                    ws.close();
                    break;
                case 'error':
                    updateStatus(`Error: ${msg.message}`, "--error");
                    ws.close();
                    break;
            }
        }

        async function uploadFileChunks() {
            let offset = 0;
            const reader = new FileReader();

            reader.onload = e => {
                if (ws.readyState !== WebSocket.OPEN) return;
                ws.send(e.target.result);
                offset += e.target.result.byteLength;
                if (offset < selectedFile.size) readNextChunk();
            };

            const readNextChunk = () => reader.readAsArrayBuffer(selectedFile.slice(offset, offset + CHUNK_SIZE));
            readNextChunk();
        }

        function formatSize(bytes) {
            if (bytes === 0) return '0 B';
            const k = 1024;
            const sizes = ['B', 'KB', 'MB', 'GB'];
            const i = Math.floor(Math.log(bytes) / Math.log(k));
            return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + sizes[i];
        }
