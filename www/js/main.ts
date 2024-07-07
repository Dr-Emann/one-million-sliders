const PADDING_ROWS = 6;
const NUM_VALUES = 1000000;

let visibleWidth: number = 0;
let visibleHeight: number = 0;
let fullHeight: number = 0;

let cbHeight: number = 0;
let cbWidth: number = 0;

let numCols: number = 0;
let numRows: number = 0;

let visibleRows: number = 0;
let firstVisibleRow: number = 0;
let renderedRows: Element[] = []
const content = document.getElementById('content')!;
const contentContainer = document.getElementById('content-container')!;

const data = new Uint8Array(NUM_VALUES / 8)

function setBit(n: number, value: boolean = true): void {
    let changed = false
    if (value) {
        changed = (data[n >> 3] & (1 << (n & 7))) === 0
        data[n >> 3] |= (1 << (n & 7))
    } else {
        changed = (data[n >> 3] & (1 << (n & 7))) !== 0
        data[n >> 3] &= ~(1 << (n & 7))
    }
    if (!changed) {
        return
    }
    console.log("Set bit", n, "to", value)
    const firstRenderedRow = Math.max(0, firstVisibleRow - PADDING_ROWS);
    const row = Math.floor(n / numCols) - firstRenderedRow;
    if (row >= 0 && row < renderedRows.length) {
        const cb = renderedRows[row].children[n % numCols].children[0] as HTMLInputElement;
        cb.checked = value;
    }
}

function getBit(n: number): boolean {
    return (data[n >> 3] & (1 << (n & 7))) !== 0
}

function onResize(): void {
    const firstVisibleCb = firstVisibleRow * numCols;
    // Find the number of input elements that can fit in the container
    content.textContent = '';
    content.style.removeProperty('height')
    renderedRows = []
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
    content.style.height = `${fullHeight}px`

    visibleHeight = contentContainer.clientHeight;
    visibleRows = Math.ceil(visibleHeight / cbHeight);
    firstVisibleRow = Math.floor(firstVisibleCb / numCols);
    contentContainer.scrollTop = firstVisibleRow * cbHeight;
    doScroll(true)
}

function doScroll(force: boolean): void {
    const scrollTop = contentContainer.scrollTop;
    const oldFirstRenderedRow = Math.max(0, firstVisibleRow - PADDING_ROWS);
    const oldLastRenderedRow = Math.min(numRows, firstVisibleRow + visibleRows + PADDING_ROWS);

    firstVisibleRow = Math.floor(scrollTop / cbHeight);
    const newFirstRenderedRow = Math.max(0, firstVisibleRow - PADDING_ROWS);
    const newLastRenderedRow = Math.min(numRows, firstVisibleRow + visibleRows + PADDING_ROWS);

    if (!force) {
        if (newFirstRenderedRow === oldFirstRenderedRow) {
            return;
        } else if (newFirstRenderedRow > oldFirstRenderedRow) {
            let removedRows = renderedRows.splice(0, newFirstRenderedRow - oldFirstRenderedRow);
            removedRows.forEach(row => content.removeChild(row));
        } else if (newFirstRenderedRow < oldFirstRenderedRow) {
            let toRemove = oldLastRenderedRow - newLastRenderedRow;
            let removedRows = renderedRows.splice(-toRemove);
            removedRows.forEach(row => content.removeChild(row));

            for (let i = newFirstRenderedRow; i < Math.min(oldFirstRenderedRow, newLastRenderedRow); i++) {
                const row = makeRow(i);
                content.insertBefore(row, content.firstChild);
                renderedRows.splice(i - newFirstRenderedRow, 0, row);
            }
        }
    } else {
        if (renderedRows.length !== 0) {
            console.warn("force with non-empty renderedRows")
        }
    }

    const roundedFirstCheckbox = (((newFirstRenderedRow * numCols) / 512) | 0) * 512;
    const roundedLastCheckbox = (((newLastRenderedRow * numCols + 511) / 512) | 0) * 512;
    if (roundedFirstCheckbox !== eventSourceStart || roundedLastCheckbox !== eventSourceEnd) {
        eventSourceStart = roundedFirstCheckbox;
        eventSourceEnd = roundedLastCheckbox;
        createEventSource()
    }

    for (let i = renderedRows.length; i < newLastRenderedRow - newFirstRenderedRow; i++) {
        const row = makeRow(i + newFirstRenderedRow);
        content.appendChild(row);
        renderedRows.push(row)
    }
}

function makeCb(checked: boolean = false): HTMLDivElement {
    const cb = document.createElement('div');
    cb.className = 'cb';
    const input = cb.appendChild(document.createElement('input'));
    input.type = 'checkbox';
    input.checked = checked;
    return cb;

}

function makeRow(n: number): HTMLDivElement {
    const row = document.createElement('div');
    row.className = 'cb-row';
    for (let i = 0; i < numCols; i++) {
        let bitIdx = n * numCols + i;
        if (bitIdx >= NUM_VALUES) {
            break
        }
        const cb = makeCb(getBit(bitIdx));
        cb.onchange = (ev) => {
            setBit(bitIdx, (ev.currentTarget as HTMLInputElement).checked);
            fetch(`http://localhost:8000/toggle/${bitIdx}`, {
                method: 'POST',
            })
        }
        row.appendChild(cb);
    }
    row.style.top = `${n * cbHeight}px`;
    return row;
}

function updateCount(count: number): void {
    const countEl = document.getElementById('count')!;
    countEl.textContent = count.toString();
}

function handleUpdate(offset: number, base64Data: string): void {
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

let eventSourceStart = 0
let eventSourceEnd = 0
function createEventSource(): void {
    eventSource?.close()
    eventSource = new EventSource(`http://localhost:8000/updates?start=${eventSourceStart}&end=${eventSourceEnd}`);
    eventSource.addEventListener("error", createEventSource)
    eventSource.addEventListener("count", (ev) => updateCount(parseInt(ev.data)))
    eventSource.addEventListener("update", (ev) => handleUpdate(parseInt(ev.lastEventId), ev.data))
}

let eventSource: EventSource | null = null

window.addEventListener('resize', onResize)
window.addEventListener('load', onResize)
contentContainer.addEventListener("scroll", () => doScroll(false))