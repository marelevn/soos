package io.github.marelevn.soos

import java.io.File
import java.nio.file.Files
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ModelTest {
    private fun folder(): File = Files.createTempDirectory("soos-model").toFile()

    @Test
    fun theSaveReadsBackAsItWasWritten() {
        val saved = Saved(
            tabs = listOf(SavedTab(3, "Rent: 1800\n\"quoted\" €5 😀"), SavedTab(7, "")),
            active = 1,
            converters = listOf(ConverterRow("teu", "TEU, teus", "cbm", "33.2")),
            decimalComma = true,
            highPrecision = true,
        )
        assertEquals(saved, decode(encode(saved)))
    }

    @Test
    fun aSaveMissingFieldsKeepsTheRest() {
        val saved = decode("""{"tabs":[{"id":2,"text":"1 + 1"}]}""")
        assertEquals(Saved(tabs = listOf(SavedTab(2, "1 + 1"))), saved)
    }

    @Test
    fun somethingThatIsNotASaveIsNull() {
        assertNull(decode("not json"))
        assertNull(decode("""{"tabs":[{"id":"x"}]}"""))
    }

    @Test
    fun anUnreadableSaveIsKeptNextToTheNewOne() {
        val dir = folder()
        val file = File(dir, "state.json")
        file.writeText("garbage")
        val loaded = loadSaved(file)
        assertEquals(Saved(), loaded.saved)
        assertTrue(loaded.warning!!.contains("state.json.bak"))
        assertFalse(file.exists())
        assertEquals("garbage", File(dir, "state.json.bak").readText())

        // A second one doesn't replace the first.
        file.writeText("more garbage")
        assertTrue(loadSaved(file).warning!!.contains("state.json.bak.2"))
        assertEquals("garbage", File(dir, "state.json.bak").readText())
    }

    @Test
    fun aMissingOrEmptySaveIsAFirstRun() {
        val file = File(folder(), "state.json")
        assertNull(loadSaved(file).warning)
        file.writeText("")
        assertNull(loadSaved(file).warning)
        assertTrue(file.exists())
    }

    @Test
    fun anAtomicWriteReplacesAndLeavesNoTempFile() {
        val dir = folder()
        val file = File(dir, "state.json")
        writeAtomically(file, "one")
        writeAtomically(file, "two")
        assertEquals("two", file.readText())
        assertEquals(listOf("state.json"), dir.list()!!.toList())
    }

    @Test
    fun aTabIsTitledByItsFirstLine() {
        assertEquals("Tab 1", tabTitle("", 0))
        assertEquals("Tab 3", tabTitle("\n  \n", 2))
        assertEquals("Trip", tabTitle("\n# Trip\n1 + 1", 0))
        assertEquals("note", tabTitle("//// note", 0))
        assertEquals("Tab 2", tabTitle("###", 1))
        assertEquals("Rent: 1800", tabTitle("Rent: 1800", 0))
    }

    @Test
    fun aConverterRowIsTrimmedAndItsAliasesSplit() {
        val json = ConverterRow(" teu ", "TEU, , teus ", " cbm", "33.2 ").toJson()
        assertEquals("teu", json.getString("unit"))
        assertEquals("""["TEU","teus"]""", json.getJSONArray("aliases").toString())
        assertEquals("cbm", json.getString("base"))
        assertEquals("33.2", json.getString("factor"))
    }

    @Test
    fun theBridgesAnswerIsRead() {
        val calculated = parseCalculated(
            """{"converters":[{"ok":"33.2 m^3"},{"error":"unknown unit"}],
                "lines":[null,{"text":"10","copy":"10","error":null,"total":true},
                         {"text":"unknown x","copy":"unknown x","error":"unknown identifier 'x'","total":false}]}""",
        )
        assertEquals(ConverterStatus("33.2 m^3", null), calculated.converters[0])
        assertEquals(ConverterStatus(null, "unknown unit"), calculated.converters[1])
        assertNull(calculated.lines[0])
        assertEquals(Cell("10", "10", null, true), calculated.lines[1])
        assertEquals("unknown identifier 'x'", calculated.lines[2]!!.error)
    }
}
