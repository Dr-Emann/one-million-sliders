"use strict";
const PADDING_ROWS = 6;
const NUM_VALUES = 1000000;
let visibleWidth = 0;
let visibleHeight = 0;
let fullHeight = 0;
let cbHeight = 0;
let cbWidth = 0;
let numCols = 0;
let numRows = 0;
let visibleRows = 0;
let firstVisibleRow = 0;
let renderedRows = [];
const content = document.getElementById('content');
const contentContainer = document.getElementById('content-container');
const data = new Uint8Array(NUM_VALUES / 8);
function setBit(n, value = true) {
    let changed = false;
    if (value) {
        changed = (data[n >> 3] & (1 << (n & 7))) === 0;
        data[n >> 3] |= (1 << (n & 7));
    }
    else {
        changed = (data[n >> 3] & (1 << (n & 7))) !== 0;
        data[n >> 3] &= ~(1 << (n & 7));
    }
    if (!changed) {
        return;
    }
    console.log("Set bit", n, "to", value);
    const firstRenderedRow = Math.max(0, firstVisibleRow - PADDING_ROWS);
    const row = Math.floor(n / numCols) - firstRenderedRow;
    if (row >= 0 && row < renderedRows.length) {
        const cb = renderedRows[row].children[n % numCols].children[0];
        cb.checked = value;
    }
}
function getBit(n) {
    return (data[n >> 3] & (1 << (n & 7))) !== 0;
}
function onResize() {
    const firstVisibleCb = firstVisibleRow * numCols;
    // Find the number of input elements that can fit in the container
    content.textContent = '';
    content.style.removeProperty('height');
    renderedRows = [];
    visibleWidth = content.clientWidth;
    // Create a checkbox, then measure its width and height
    const item = makeCb();
    content.appendChild(item);
    cbWidth = item.clientWidth;
    cbHeight = item.clientHeight;
    content.removeChild(item);
    numCols = Math.floor(visibleWidth / cbWidth);
    numRows = Math.ceil(1000000 / numCols);
    fullHeight = numRows * cbHeight;
    content.style.height = `${fullHeight}px`;
    visibleHeight = contentContainer.clientHeight;
    visibleRows = Math.ceil(visibleHeight / cbHeight);
    firstVisibleRow = Math.floor(firstVisibleCb / numCols);
    contentContainer.scrollTop = firstVisibleRow * cbHeight;
    doScroll(true);
}
function doScroll(force) {
    const scrollTop = contentContainer.scrollTop;
    const oldFirstRenderedRow = Math.max(0, firstVisibleRow - PADDING_ROWS);
    const oldLastRenderedRow = Math.min(numRows, firstVisibleRow + visibleRows + PADDING_ROWS);
    firstVisibleRow = Math.floor(scrollTop / cbHeight);
    const newFirstRenderedRow = Math.max(0, firstVisibleRow - PADDING_ROWS);
    const newLastRenderedRow = Math.min(numRows, firstVisibleRow + visibleRows + PADDING_ROWS);
    if (!force) {
        if (newFirstRenderedRow === oldFirstRenderedRow) {
            return;
        }
        else if (newFirstRenderedRow > oldFirstRenderedRow) {
            let removedRows = renderedRows.splice(0, newFirstRenderedRow - oldFirstRenderedRow);
            removedRows.forEach(row => content.removeChild(row));
        }
        else if (newFirstRenderedRow < oldFirstRenderedRow) {
            let toRemove = oldLastRenderedRow - newLastRenderedRow;
            let removedRows = renderedRows.splice(-toRemove);
            removedRows.forEach(row => content.removeChild(row));
            for (let i = newFirstRenderedRow; i < Math.min(oldFirstRenderedRow, newLastRenderedRow); i++) {
                const row = makeRow(i);
                content.insertBefore(row, content.firstChild);
                renderedRows.splice(i - newFirstRenderedRow, 0, row);
            }
        }
    }
    else {
        if (renderedRows.length !== 0) {
            console.warn("force with non-empty renderedRows");
        }
    }
    const roundedFirstCheckbox = (((newFirstRenderedRow * numCols) / 512) | 0) * 512;
    const roundedLastCheckbox = (((newLastRenderedRow * numCols + 511) / 512) | 0) * 512;
    if (roundedFirstCheckbox !== eventSourceStart || roundedLastCheckbox !== eventSourceEnd) {
        eventSourceStart = roundedFirstCheckbox;
        eventSourceEnd = roundedLastCheckbox;
        createEventSource();
    }
    for (let i = renderedRows.length; i < newLastRenderedRow - newFirstRenderedRow; i++) {
        const row = makeRow(i + newFirstRenderedRow);
        content.appendChild(row);
        renderedRows.push(row);
    }
}
function makeCb(checked = false) {
    const cb = document.createElement('div');
    cb.className = 'cb';
    const input = cb.appendChild(document.createElement('input'));
    input.type = 'checkbox';
    input.checked = checked;
    return cb;
}
function makeRow(n) {
    const row = document.createElement('div');
    row.className = 'cb-row';
    for (let i = 0; i < numCols; i++) {
        let bitIdx = n * numCols + i;
        if (bitIdx >= NUM_VALUES) {
            break;
        }
        const cb = makeCb(getBit(bitIdx));
        cb.onchange = (ev) => {
            setBit(bitIdx, ev.currentTarget.checked);
            fetch(`http://localhost:8000/toggle/${bitIdx}`, {
                method: 'POST',
            });
        };
        row.appendChild(cb);
    }
    row.style.top = `${n * cbHeight}px`;
    return row;
}
function updateCount(count) {
    const countEl = document.getElementById('count');
    countEl.textContent = count.toString();
}
function handleUpdate(offset, base64Data) {
    const data = atob(base64Data);
    let i = offset;
    for (let j = 0; j < data.length; j++) {
        const byte = data.charCodeAt(j);
        for (let k = 0; k < 8; k++) {
            setBit(i, (byte & (1 << k)) !== 0);
            i++;
        }
    }
}
let eventSourceStart = 0;
let eventSourceEnd = 0;
function createEventSource() {
    eventSource === null || eventSource === void 0 ? void 0 : eventSource.close();
    eventSource = new EventSource(`http://localhost:8000/updates?start=${eventSourceStart}&end=${eventSourceEnd}`);
    eventSource.addEventListener("error", createEventSource);
    eventSource.addEventListener("count", (ev) => updateCount(parseInt(ev.data)));
    eventSource.addEventListener("update", (ev) => handleUpdate(parseInt(ev.lastEventId), ev.data));
}
let eventSource = null;
window.addEventListener('resize', onResize);
window.addEventListener('load', onResize);
contentContainer.addEventListener("scroll", () => doScroll(false));
//# sourceMappingURL=main.js.map