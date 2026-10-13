package io.github.marelevn.soos

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.relocation.BringIntoViewRequester
import androidx.compose.foundation.relocation.bringIntoViewRequester
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/** The strip is the background tinted this far toward the text colour. */
private const val TAB_BAR_CONTRAST = 0.06f

/** Rates older than this get an age warning. */
private const val RATES_OLD_AFTER_SECONDS = 24 * 60 * 60L

/** A glyph or a few characters set in the app's font at its size. */
@Composable
internal fun textStyle(color: Color) = TextStyle(fontFamily = mono(), fontSize = FontSize, color = color)

/** The tabs across the top, which reach the strip's bottom edge, where the active one meets its page. */
@Composable
fun TabStrip(state: AppState, palette: Palette) {
    val strip = lerp(palette.background, palette.plain, TAB_BAR_CONTRAST)
    Row(
        Modifier
            .fillMaxWidth()
            .background(strip)
            .statusBarsPadding()
            .padding(top = 8.dp, start = 8.dp, end = 8.dp)
            .horizontalScroll(rememberScrollState()),
        verticalAlignment = Alignment.Bottom,
    ) {
        state.tabs.forEachIndexed { index, tab ->
            val selected = index == state.active
            val bringIntoView = remember { BringIntoViewRequester() }
            if (selected) LaunchedEffect(Unit) { bringIntoView.bringIntoView() }
            Row(
                Modifier
                    .height(44.dp)
                    .widthIn(max = 200.dp)
                    .clip(RoundedCornerShape(topStart = 8.dp, topEnd = 8.dp))
                    .background(if (selected) palette.background else Color.Transparent)
                    .bringIntoViewRequester(bringIntoView)
                    .selectable(selected = selected, role = Role.Tab) { state.select(index) }
                    .padding(start = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                BasicText(
                    tabTitle(tab.text, index),
                    style = textStyle(if (selected) palette.plain else palette.comment),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
                Box(
                    Modifier
                        .size(44.dp)
                        .clickable(role = Role.Button, onClickLabel = "Close tab") { state.closeTab(index) },
                    contentAlignment = Alignment.Center,
                ) {
                    BasicText("×", style = textStyle(palette.comment))
                }
            }
        }
        if (state.tabs.size < MAX_TABS) {
            Box(
                Modifier
                    .size(44.dp)
                    .clickable(role = Role.Button, onClickLabel = "New tab") { state.addTab() },
                contentAlignment = Alignment.Center,
            ) {
                BasicText("+", style = textStyle(palette.comment))
            }
        }
    }
}

/**
 * What the last tap has to say: a result that was copied, in full (the
 * column may have cut it), or an error's whole message. It spans the width
 * and wraps, which the status bar can't.
 */
@Composable
fun Toast(state: AppState, palette: Palette, modifier: Modifier = Modifier) {
    val message = state.message ?: return
    BasicText(
        message.text,
        style = textStyle(if (message.error) palette.error else palette.plain),
        maxLines = 4,
        overflow = TextOverflow.Ellipsis,
        modifier = modifier
            .padding(8.dp)
            .clip(RoundedCornerShape(8.dp))
            .background(lerp(palette.background, palette.plain, 0.14f))
            .padding(horizontal = 16.dp, vertical = 12.dp)
            // So a screen reader says it, as it can't see a toast come and go.
            .semantics { liveRegion = LiveRegionMode.Polite },
    )
}

/** Offers to bring back the tab just closed. */
@Composable
fun UndoBar(state: AppState, palette: Palette, modifier: Modifier = Modifier) {
    val closed = state.closedTab ?: return
    Row(
        modifier
            .padding(8.dp)
            .clip(RoundedCornerShape(8.dp))
            .background(lerp(palette.background, palette.plain, 0.14f))
            .padding(start = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        BasicText(
            "Closed ${tabTitle(closed.tab.text, closed.index)}",
            style = textStyle(palette.plain),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f, fill = false),
        )
        Box(
            Modifier
                .sizeIn(minWidth = 64.dp, minHeight = 48.dp)
                .clickable(role = Role.Button) { state.undoClose() }
                .padding(horizontal = 12.dp),
            contentAlignment = Alignment.Center,
        ) {
            BasicText("Undo", style = textStyle(palette.keyword))
        }
    }
}

/**
 * The warning from launch, or the exchange rates' state, or the version; and
 * the toggles on the right. Nothing here can be longer than a few words,
 * since the toggles take most of the width: what a tap has to say goes in
 * [Toast].
 */
@Composable
fun StatusBar(state: AppState, palette: Palette) {
    Row(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        val (text, color) = statusText(state, palette)
        BasicText(
            text,
            style = textStyle(color),
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier
                .weight(1f)
                .then(
                    if (state.loadWarning != null) {
                        Modifier.clickable(onClickLabel = "Dismiss") { state.dismissWarning() }
                    } else {
                        Modifier
                    },
                ),
        )
        StatusToggle(
            glyph = "↔",
            on = state.showConverters,
            onColor = palette.keyword,
            description = "Converters: define your own units",
            palette = palette,
        ) { state.showConverters = !state.showConverters }
        StatusToggle(
            glyph = if (state.decimalComma) "0,1" else "0.1",
            on = state.decimalComma,
            onColor = palette.result,
            description = if (state.decimalComma) {
                "Decimal comma is on: 1.234,5"
            } else {
                "Decimal point is on: 1,234.5"
            },
            palette = palette,
        ) { state.toggleDecimalComma() }
        StatusToggle(
            glyph = "±",
            on = state.highPrecision,
            onColor = palette.result,
            description = if (state.highPrecision) {
                "High precision is on: currencies keep every digit"
            } else {
                "High precision is off: currencies are rounded"
            },
            palette = palette,
        ) { state.toggleHighPrecision() }
    }
}

private fun statusText(state: AppState, palette: Palette): Pair<String, Color> {
    state.loadWarning?.let { return it to palette.error }
    val age = state.ratesAge
    return when {
        age < 0 -> "rates offline" to palette.comment
        age > RATES_OLD_AFTER_SECONDS -> {
            val days = age / RATES_OLD_AFTER_SECONDS
            "rates ${if (days == 1L) "1 day" else "$days days"} old" to palette.comment
        }
        else -> "v${state.version}" to palette.comment
    }
}

/** A status-bar symbol in `palette.comment`, in `onColor` while on. */
@Composable
private fun StatusToggle(
    glyph: String,
    on: Boolean,
    onColor: Color,
    description: String,
    palette: Palette,
    onToggle: () -> Unit,
) {
    Box(
        Modifier
            .sizeIn(minWidth = 48.dp, minHeight = 48.dp)
            .semantics { contentDescription = description }
            .toggleable(value = on, role = Role.Switch) { onToggle() },
        contentAlignment = Alignment.Center,
    ) {
        BasicText(glyph, style = textStyle(if (on) onColor else palette.comment))
    }
}
