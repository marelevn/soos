package io.github.marelevn.soos

import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicText
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp

/**
 * The converters table: the user's own units, kept apart from the document.
 * Each row is `unit` = `factor` × `base`, and soos-core checks it on every
 * recalculation; what it says is under the row.
 */
@Composable
fun ConvertersScreen(state: AppState, palette: Palette, modifier: Modifier = Modifier) {
    val statuses = state.calculated?.converters ?: emptyList()
    // So the hint reads the way the user types a decimal.
    val factorHint = if (state.decimalComma) "33,2" else "33.2"
    LazyColumn(
        modifier.fillMaxSize(),
        contentPadding = PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(24.dp),
    ) {
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                BasicText(
                    "Converters",
                    style = textStyle(palette.plain).copy(fontWeight = FontWeight.Bold),
                    modifier = Modifier.weight(1f),
                )
                Button("Done", palette.keyword, onClickLabel = "Close the converters") {
                    state.showConverters = false
                }
            }
            BasicText(
                "Your own units: 1 unit = factor × base.",
                style = textStyle(palette.comment),
                modifier = Modifier.padding(top = 4.dp),
            )
        }
        itemsIndexed(state.converters) { index, row ->
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Field(row.unit, "teu", palette, Modifier.weight(1f)) {
                        state.setConverter(index, row.copy(unit = it))
                    }
                    Field(row.aliases, "TEU, teus", palette, Modifier.weight(1f)) {
                        state.setConverter(index, row.copy(aliases = it))
                    }
                }
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Field(row.base, "cbm", palette, Modifier.weight(1f)) {
                        state.setConverter(index, row.copy(base = it))
                    }
                    Field(row.factor, factorHint, palette, Modifier.weight(1f)) {
                        state.setConverter(index, row.copy(factor = it))
                    }
                }
                Row(verticalAlignment = Alignment.CenterVertically) {
                    val (text, color) = rowStatus(row, statuses.getOrNull(index), palette)
                    BasicText(
                        text,
                        style = textStyle(color),
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    Button("↑", palette.plain, enabled = index > 0, onClickLabel = "Move up") {
                        state.moveConverter(index, index - 1)
                    }
                    Button(
                        "↓",
                        palette.plain,
                        enabled = index + 1 < state.converters.size,
                        onClickLabel = "Move down",
                    ) { state.moveConverter(index, index + 1) }
                    Button("−", palette.plain, onClickLabel = "Remove") {
                        state.removeConverter(index)
                    }
                }
            }
        }
        item {
            if (state.converters.size >= MAX_CONVERTERS) {
                BasicText("$MAX_CONVERTERS converters max", style = textStyle(palette.comment))
            } else {
                Button("+ Add converter", palette.keyword) { state.addConverter() }
            }
        }
    }
}

/** Grey, not red, while a row is still being filled in. */
private fun rowStatus(
    row: ConverterRow,
    status: ConverterStatus?,
    palette: Palette,
): Pair<String, androidx.compose.ui.graphics.Color> {
    val missing = listOfNotNull(
        "unit".takeIf { row.unit.isBlank() },
        "base".takeIf { row.base.isBlank() },
        "factor".takeIf { row.factor.isBlank() },
    )
    return when {
        missing.isNotEmpty() -> "needs ${missing.joinToString(", ")}" to palette.comment
        status?.ok != null -> "1 ${row.unit.trim()} = ${status.ok}" to palette.result
        status?.error != null -> status.error to palette.error
        else -> "" to palette.comment
    }
}

@Composable
private fun Field(
    value: String,
    hint: String,
    palette: Palette,
    modifier: Modifier,
    onChange: (String) -> Unit,
) {
    BasicTextField(
        value = value,
        onValueChange = onChange,
        modifier = modifier
            .border(1.dp, palette.comment, RoundedCornerShape(8.dp))
            .padding(12.dp),
        singleLine = true,
        textStyle = textStyle(palette.plain),
        cursorBrush = SolidColor(palette.plain),
        keyboardOptions = KeyboardOptions(
            capitalization = KeyboardCapitalization.None,
            autoCorrectEnabled = false,
        ),
        decorationBox = { field ->
            Box {
                if (value.isEmpty()) BasicText(hint, style = textStyle(palette.comment))
                field()
            }
        },
    )
}

@Composable
private fun Button(
    label: String,
    color: androidx.compose.ui.graphics.Color,
    enabled: Boolean = true,
    onClickLabel: String? = null,
    onClick: () -> Unit,
) {
    Box(
        Modifier
            .sizeIn(minWidth = 48.dp, minHeight = 48.dp)
            .clickable(enabled = enabled, role = Role.Button, onClickLabel = onClickLabel, onClick = onClick)
            .padding(horizontal = 8.dp),
        contentAlignment = Alignment.Center,
    ) {
        BasicText(label, style = textStyle(if (enabled) color else color.copy(alpha = 0.35f)))
    }
}
