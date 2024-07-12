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

const data = new Uint8Array(NUM_VALUES)

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
}

function getBit(n: number): boolean {
    return (data[n >> 3] & (1 << (n & 7))) !== 0
}

function setByte(n: number, value: number): void {
    value = value & 0xFF
    let prev = getByte(n)
    data[n] = value
    const changed = data[n] != prev
    if (!changed) {
        return
    }
    console.log("Set byte", n, "to", value)
    const firstRenderedRow = Math.max(0, firstVisibleRow - PADDING_ROWS);
    const row = Math.floor(n / numCols) - firstRenderedRow;
    if (row >= 0 && row < renderedRows.length) {
        const cb = renderedRows[row].children[n % numCols].children[0] as HTMLInputElement;
        cb.value = value.toString()
    }
}

function getByte(n: number): number {
    return data[n]
}

function onResize(): void {
    const firstVisibleCb = firstVisibleRow * numCols;
    // Find the number of input elements that can fit in the container
    content.textContent = '';
    content.style.removeProperty('height')
    renderedRows = []
    visibleWidth = content.clientWidth;

    // Create a checkbox, then measure its width and height
    const item = makeSlider(0);
    content.appendChild(item);
    cbWidth = item.clientWidth;
    cbHeight = item.clientHeight;
    content.removeChild(item);

    numCols = Math.floor(visibleWidth / cbWidth);
    // Ensure at least 4 columns
    if (numCols < 4) {
        numCols = 4
    }
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

function makeSlider(value: number): HTMLDivElement {
    const slider = document.createElement('div')
    slider.className = 'slider'
    const input = slider.appendChild(document.createElement('input'))
    input.type = 'range'
    input.min = '0'
    input.max = '255'
    input.value = value.toString()
    return slider
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
        let byteIdx = n * numCols + i;
        if (byteIdx >= NUM_VALUES) {
            break
        }
        const slider = makeSlider(getByte(byteIdx))
        slider.onchange = (ev) => {
            const target = ev.currentTarget as HTMLDivElement
            const inputElem = target.children[0] as HTMLInputElement
            const value = parseInt(inputElem.value);
            setByte(byteIdx, value)
            fetch(`set_byte/${byteIdx}/${value}`, {
                method: 'POST',
            })
        }
        row.appendChild(slider)
    }
    row.style.top = `${n * cbHeight}px`;
    return row;
}

function updateSum(sum: number): void {
    const countEl = document.getElementById('avg')!;
    const percent = sum / 255 / NUM_VALUES * 100;
    countEl.textContent = `${percent.toFixed(7)}%`
}

function handleUpdate(offset: number, base64Data: string): void {
    const data = atob(base64Data);
    // Offset is in bits, so divide by 8 to get bytes
    let i = offset / 8;
    for (let j = 0; j < data.length; j++) {
        const byte = data.charCodeAt(j);
        setByte(i + j, byte)
    }
}

let eventSourceStart = 0
let eventSourceEnd = 0
function createEventSource(): void {
    eventSource?.close()
    // Units are in bytes now
    eventSource = new EventSource(`updates?start=${eventSourceStart * 8}&end=${eventSourceEnd * 8}`);
    eventSource.addEventListener("error", () => {
        eventSource?.close()
        setTimeout(createEventSource, 500)
    })
    eventSource.addEventListener("sum", (ev) => updateSum(parseFloat(ev.data)))
    eventSource.addEventListener("update", (ev) => handleUpdate(parseInt(ev.lastEventId), ev.data))
}

let eventSource: EventSource | null = null

window.addEventListener('resize', onResize)
window.addEventListener('load', onResize)
contentContainer.addEventListener("scroll", () => doScroll(false))