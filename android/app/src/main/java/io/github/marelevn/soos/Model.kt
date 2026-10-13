package io.github.marelevn.soos

import java.io.File
import java.io.FileOutputStream
import java.nio.file.Files
import java.nio.file.StandardCopyOption
import org.json.JSONArray
import org.json.JSONObject

/** Tabs across the top, as on the desktop. */
const val MAX_TABS = 9

/** The same as soos-core's `MAX_CONVERTERS`: each costs a define and a check per recalculation. */
const val MAX_CONVERTERS = 64

/** One row of the converters table, as typed: `unit` = `factor` × `base`. */
data class ConverterRow(
    val unit: String = "",
    /** Comma-separated. */
    val aliases: String = "",
    val base: String = "",
    val factor: String = "",
) {
    /** soos-core's `RawConverter`, trimmed and split as the desktop's `to_raw` does. */
    fun toJson(): JSONObject = JSONObject()
        .put("unit", unit.trim())
        .put("aliases", JSONArray(aliases.split(',').map { it.trim() }.filter { it.isNotEmpty() }))
        .put("base", base.trim())
        .put("factor", factor.trim())
}

/** One tab as saved. `id` is never reused, so a closed tab can come back as itself. */
data class SavedTab(val id: Long, val text: String)

/** What's kept between runs. Every field has a default, so a missing one never discards the rest. */
data class Saved(
    val tabs: List<SavedTab> = emptyList(),
    val active: Int = 0,
    val converters: List<ConverterRow> = emptyList(),
    val decimalComma: Boolean = false,
    val highPrecision: Boolean = false,
)

fun encode(saved: Saved): String = JSONObject()
    .put("active", saved.active)
    .put("decimalComma", saved.decimalComma)
    .put("highPrecision", saved.highPrecision)
    .put(
        "tabs",
        JSONArray(saved.tabs.map { JSONObject().put("id", it.id).put("text", it.text) }),
    )
    .put(
        "converters",
        JSONArray(
            saved.converters.map {
                JSONObject()
                    .put("unit", it.unit)
                    .put("aliases", it.aliases)
                    .put("base", it.base)
                    .put("factor", it.factor)
            },
        ),
    )
    .toString()

/** `null` if `json` isn't a save at all. */
fun decode(json: String): Saved? = try {
    val root = JSONObject(json)
    val tabs = root.optJSONArray("tabs")
    val converters = root.optJSONArray("converters")
    Saved(
        tabs = List(tabs?.length() ?: 0) {
            tabs!!.getJSONObject(it).let { tab -> SavedTab(tab.getLong("id"), tab.getString("text")) }
        },
        active = root.optInt("active", 0),
        converters = List(converters?.length() ?: 0) {
            converters!!.getJSONObject(it).let { row ->
                ConverterRow(
                    row.optString("unit"),
                    row.optString("aliases"),
                    row.optString("base"),
                    row.optString("factor"),
                )
            }
        },
        decimalComma = root.optBoolean("decimalComma", false),
        highPrecision = root.optBoolean("highPrecision", false),
    )
} catch (_: org.json.JSONException) {
    null
}

/** What [loadSaved] found: the save, and why it was set aside if it couldn't be read. */
data class Loaded(val saved: Saved, val warning: String?)

/**
 * The save in `file`. One that can't be read is moved to `.bak` (or `.bak.2`,
 * never over an older backup), because the next autosave would otherwise
 * replace every tab with a blank one. A missing or empty file is a first run.
 */
fun loadSaved(file: File): Loaded {
    if (!file.exists() || file.length() == 0L) return Loaded(Saved(), null)
    decode(file.readText())?.let { return Loaded(it, null) }
    val backup = generateSequence(1) { it + 1 }
        .map { File(file.parentFile, if (it == 1) "${file.name}.bak" else "${file.name}.bak.$it") }
        .first { !it.exists() }
    val warning = if (file.renameTo(backup)) {
        "Couldn't read your saved tabs. The old file is kept as ${backup.name}."
    } else {
        "Couldn't read your saved tabs, or move the file aside."
    }
    return Loaded(Saved(), warning)
}

/**
 * Replaces `file` with `text` so a crash or a kill leaves the old file or
 * the new one, never half of one: write a temp file beside it, flush it to
 * disk, move it over.
 */
fun writeAtomically(file: File, text: String) {
    val temp = File(file.parentFile, ".${file.name}.tmp")
    FileOutputStream(temp).use {
        it.write(text.toByteArray())
        it.fd.sync()
    }
    Files.move(
        temp.toPath(),
        file.toPath(),
        StandardCopyOption.ATOMIC_MOVE,
        StandardCopyOption.REPLACE_EXISTING,
    )
}

/**
 * A tab's title: its first non-empty line without a leading `#` or `//`, or
 * "Tab N" while it has none. The desktop's `tab_title`.
 */
fun tabTitle(text: String, position: Int): String {
    val line = text.lineSequence().map { it.trim() }.firstOrNull { it.isNotEmpty() }
    var title = line?.trimStart('#') ?: ""
    while (title.startsWith("//")) title = title.removePrefix("//")
    return title.trim().ifEmpty { "Tab ${position + 1}" }
}
