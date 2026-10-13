package io.github.marelevn.soos

import android.app.Application
import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import java.io.File
import java.io.IOException
import java.util.Collections
import java.util.concurrent.Executors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** A file in the app's folder, there while the first recalculation after a launch runs. */
private const val LAUNCH_MARKER = "recalculating"

/** One tab's document. */
class Tab(val id: Long, text: String) {
    /** Text and selection together, so switching tabs keeps the caret. */
    var value by mutableStateOf(TextFieldValue(text, TextRange(text.length)))
    val text: String get() = value.text
}

/** A line of the status bar, until it expires. */
class Message(val id: Long, val text: String, val error: Boolean)

/** The tab closed last, for the Undo bar. */
class ClosedTab(val id: Long, val index: Int, val tab: Tab)

private class Request(
    val generation: Long,
    val text: String,
    val converters: List<ConverterRow>,
    val highPrecision: Boolean,
    val decimalComma: Boolean,
)

/**
 * Everything the screens show, and what changes it. The whole document is
 * recalculated on every edit, and a line may take up to soos-core's 2-second
 * limit, so it runs on its own thread, newest edit only: a newer request
 * cancels the one running (see `Engine.cancelOlderThan`), and one still
 * queued is replaced.
 */
class AppState(app: Application) : AndroidViewModel(app) {
    private val stateFile = File(app.filesDir, "state.json")
    private val marker = File(app.filesDir, LAUNCH_MARKER)

    @Suppress("DEPRECATION")
    val version: String = app.packageManager.getPackageInfo(app.packageName, 0).versionName ?: ""

    /** Never empty; closing the last tab leaves an empty one. */
    var tabs by mutableStateOf<List<Tab>>(emptyList())
        private set

    /** Always a valid index into [tabs]. */
    var active by mutableIntStateOf(0)
        private set
    var converters by mutableStateOf<List<ConverterRow>>(emptyList())
        private set

    /** `,` is the decimal mark and `.` groups thousands. The saved text is as typed. */
    var decimalComma by mutableStateOf(false)
        private set
    var highPrecision by mutableStateOf(false)
        private set
    var showConverters by mutableStateOf(false)

    /** The newest results, possibly for an older version of the text while a recalculation runs. */
    var calculated by mutableStateOf<Calculated?>(null)
        private set

    /** The newest recalculation asked for, and the one [calculated] is from. */
    var requested by mutableLongStateOf(0)
        private set
    var received by mutableLongStateOf(0)
        private set

    /** Counts changes to the text, so the editor can hold back an error on the line being typed. */
    var edits by mutableIntStateOf(0)
        private set

    /** Seconds, or -1 with no rates; as of the last [refreshRates]. */
    var ratesAge by mutableLongStateOf(-1)
        private set
    var message by mutableStateOf<Message?>(null)
        private set
    var closedTab by mutableStateOf<ClosedTab?>(null)
        private set

    /** Why the saved tabs weren't used, until it's dismissed. */
    var loadWarning by mutableStateOf<String?>(null)
        private set

    val activeTab: Tab get() = tabs[active]

    private var nextTabId = 0L
    private var generation = 0L
    private var nextMessage = 0L
    private var refreshing = false
    private var saveJob: Job? = null
    private val writeLock = Any()

    /** The tab, and its text, that Soos was closed in the middle of calculating at the last launch. */
    private var paused: Pair<Long, String>? = null

    private val requests = Channel<Request>(Channel.CONFLATED)
    private val worker = Executors.newSingleThreadExecutor { Thread(it, "soos-recalc") }
        .asCoroutineDispatcher()

    init {
        val (saved, warning) = loadSaved(stateFile)
        tabs = saved.tabs.map { Tab(it.id, it.text) }.ifEmpty { listOf(Tab(0, "")) }
        nextTabId = tabs.maxOf { it.id } + 1
        active = saved.active.coerceIn(0, tabs.lastIndex)
        converters = saved.converters
        decimalComma = saved.decimalComma
        highPrecision = saved.highPrecision
        loadWarning = warning
        // A line that overflows fend's stack ends the process, and its tab is
        // saved, so Soos would close again at every launch. Found at launch,
        // the marker means the last one closed that way.
        if (marker.exists()) {
            paused = activeTab.id to activeTab.text
            loadWarning = loadWarning
                ?: "Soos closed while calculating this tab, so it waits until you edit it."
        } else {
            marker.writeText("")
        }
        ratesAge = Engine.ratesAgeSeconds()

        viewModelScope.launch(worker) {
            for (request in requests) {
                val result = Engine.calculate(
                    request.generation,
                    request.text,
                    request.converters,
                    request.highPrecision,
                    request.decimalComma,
                ) ?: continue
                withContext(Dispatchers.Main.immediate) { accept(request.generation, result) }
            }
        }
        recalculate()
    }

    override fun onCleared() {
        requests.close()
        worker.close()
    }

    private fun recalculate() {
        val tab = activeTab
        paused?.let { (id, text) ->
            if (tab.id == id && tab.text == text) {
                calculated = null
                return
            }
            if (tab.id == id) paused = null
        }
        generation += 1
        requested = generation
        Engine.cancelOlderThan(generation)
        requests.trySend(Request(generation, tab.text, converters, highPrecision, decimalComma))
    }

    private fun accept(generation: Long, result: Calculated) {
        if (generation != this.generation) return
        calculated = result
        received = generation
        marker.delete()
    }

    fun edit(tab: Tab, value: TextFieldValue) {
        val changed = value.text != tab.text
        tab.value = value
        if (!changed) return
        edits += 1
        recalculate()
        scheduleSave()
    }

    private fun tabsChanged() {
        recalculate()
        scheduleSave()
    }

    fun select(index: Int) {
        if (index == active) return
        active = index
        tabsChanged()
    }

    fun addTab() {
        if (tabs.size >= MAX_TABS) return
        tabs = tabs + Tab(nextTabId++, "")
        active = tabs.lastIndex
        tabsChanged()
    }

    fun closeTab(index: Int) {
        val closed = tabs[index]
        tabs = tabs.toMutableList().also { it.removeAt(index) }
        if (tabs.isEmpty()) tabs = listOf(Tab(nextTabId++, ""))
        active = if (index < active) active - 1 else active.coerceAtMost(tabs.lastIndex)
        val id = ++nextMessage
        closedTab = ClosedTab(id, index, closed)
        viewModelScope.launch {
            delay(UNDO_FOR_MS)
            if (closedTab?.id == id) closedTab = null
        }
        tabsChanged()
    }

    fun undoClose() {
        val closed = closedTab ?: return
        closedTab = null
        val at = closed.index.coerceAtMost(tabs.size)
        tabs = tabs.toMutableList().also { it.add(at, closed.tab) }
        active = at
        tabsChanged()
    }

    fun setConverter(index: Int, row: ConverterRow) {
        converters = converters.toMutableList().also { it[index] = row }
        convertersChanged()
    }

    fun addConverter() {
        if (converters.size >= MAX_CONVERTERS) return
        converters = converters + ConverterRow()
        convertersChanged()
    }

    fun removeConverter(index: Int) {
        converters = converters.toMutableList().also { it.removeAt(index) }
        convertersChanged()
    }

    fun moveConverter(index: Int, to: Int) {
        if (to !in converters.indices) return
        converters = converters.toMutableList().also { Collections.swap(it, index, to) }
        convertersChanged()
    }

    private fun convertersChanged() {
        recalculate()
        scheduleSave()
    }

    fun toggleDecimalComma() {
        decimalComma = !decimalComma
        recalculate()
        scheduleSave()
    }

    fun toggleHighPrecision() {
        highPrecision = !highPrecision
        recalculate()
        scheduleSave()
    }

    fun dismissWarning() {
        loadWarning = null
    }

    fun show(text: String, error: Boolean = false, forMillis: Long = COPIED_FOR_MS) {
        val id = ++nextMessage
        message = Message(id, text, error)
        viewModelScope.launch {
            delay(forMillis)
            if (message?.id == id) message = null
        }
    }

    /** Puts a result on the clipboard, in the form that reads back into Soos as the same value. */
    fun copy(cell: Cell) {
        val clipboard = getApplication<Application>().getSystemService(ClipboardManager::class.java)
        clipboard.setPrimaryClip(ClipData.newPlainText("Soos", cell.copy))
        show("Copied ${cell.copy}")
    }

    /** Downloads the exchange rates if they're stale, then recalculates the lines that use them. */
    fun refreshRates() {
        if (refreshing) return
        refreshing = true
        viewModelScope.launch {
            val before = ratesAge
            val after = withContext(Dispatchers.IO) {
                Engine.refreshRates()
                Engine.ratesAgeSeconds()
            }
            refreshing = false
            ratesAge = after
            if (after >= 0 && (before < 0 || after < before)) recalculate()
        }
    }

    private fun snapshot() = encode(
        Saved(tabs.map { SavedTab(it.id, it.text) }, active, converters, decimalComma, highPrecision),
    )

    /** A second after the last change, so typing doesn't write to the disk per keystroke. */
    private fun scheduleSave() {
        saveJob?.cancel()
        saveJob = viewModelScope.launch {
            delay(SAVE_AFTER_MS)
            val json = snapshot()
            withContext(Dispatchers.IO) { write(json) }
        }
    }

    /** For when the app leaves the screen, which may be the last the process hears of it. */
    fun saveNow() {
        saveJob?.cancel()
        write(snapshot())
    }

    private fun write(json: String) {
        try {
            synchronized(writeLock) { writeAtomically(stateFile, json) }
        } catch (e: IOException) {
            show("Couldn't save your tabs: ${e.message}", error = true, forMillis = 5000)
        }
    }

    private companion object {
        const val COPIED_FOR_MS = 1500L
        const val UNDO_FOR_MS = 6000L
        const val SAVE_AFTER_MS = 1000L
    }
}
