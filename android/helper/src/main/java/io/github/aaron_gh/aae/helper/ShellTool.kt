package io.github.aaron_gh.aae.helper

import android.view.InputDevice
import kotlin.system.exitProcess

/**
 * Commands AAE runs on the device as the shell user, which holds permissions
 * apps can't have, such as choosing the layout of a physical keyboard. AAE
 * starts it with:
 *
 *     CLASSPATH=<this APK> app_process / io.github.aaron_gh.aae.helper.ShellTool <command>
 *
 * It uses Android's hidden input manager interface through reflection, since
 * there is no shell command for keyboard layouts.
 */
object ShellTool {
    @JvmStatic
    fun main(args: Array<String>) {
        // An uncaught error would make Android end the process with nothing
        // but "Killed", so say what went wrong instead.
        try {
            run(args)
        } catch (e: Throwable) {
            val cause = (e as? java.lang.reflect.InvocationTargetException)?.targetException ?: e
            System.err.println("AAE's shell tool failed: $cause")
            exitProcess(3)
        }
    }

    private fun run(args: Array<String>) {
        when (args.firstOrNull()) {
            "methods" -> listInputManagerMethods()
            "layouts" -> listKeyboardLayouts()
            "keyboard-layout" -> useKeyboardLayout(args.getOrElse(1) { KeyboardLayouts.FULL_KEYBOARD })
            "clear-keyboard-layout" -> clearKeyboardLayout(args.getOrElse(1) { KeyboardLayouts.FULL_KEYBOARD })
            "keyboards" -> describeKeyboards()
            else -> println("Usage: ShellTool methods | layouts | keyboards | keyboard-layout [descriptor] | clear-keyboard-layout [descriptor]")
        }
    }

    private fun inputManager(): Any {
        val binder = Class.forName("android.os.ServiceManager")
            .getMethod("getService", String::class.java)
            .invoke(null, "input")
        val stub = Class.forName("android.hardware.input.IInputManager\$Stub")
        return stub.getMethod("asInterface", Class.forName("android.os.IBinder")).invoke(null, binder)!!
    }

    private fun listKeyboardLayouts() {
        val im = inputManager()
        val layouts = im.javaClass.getMethod("getKeyboardLayouts").invoke(im) as Array<*>
        for (layout in layouts) {
            val descriptor = layout!!.javaClass.getMethod("getDescriptor").invoke(layout)
            println("$descriptor: $layout")
        }
    }

    /**
     * Uses the keyboard layout [descriptor] for every physical full keyboard.
     * Prints one line per keyboard; exits with status 1 if there were none.
     */
    private fun useKeyboardLayout(descriptor: String) {
        val im = inputManager()
        val type = im.javaClass
        val ids = type.getMethod("getInputDeviceIds").invoke(im) as IntArray
        var changed = 0
        for (id in ids) {
            val device = type.getMethod("getInputDevice", Int::class.javaPrimitiveType).invoke(im, id)
                as? InputDevice ?: continue
            if (device.isVirtual || device.keyboardType != InputDevice.KEYBOARD_TYPE_ALPHABETIC) continue
            val identifier = InputDevice::class.java.getMethod("getIdentifier").invoke(device)
            val method = { name: String -> type.methods.firstOrNull { it.name == name && it.parameterCount == 2 } }
            val override = method("setKeyboardLayoutOverrideForInputDevice")
            val add = method("addKeyboardLayoutForInputDevice")
            val setCurrent = method("setCurrentKeyboardLayoutForInputDevice")
            val perInputMethod = type.methods.firstOrNull {
                it.name == "setKeyboardLayoutForInputDevice" && it.parameterCount == 5
            }
            when {
                // Android 15 and later.
                override != null -> override.invoke(im, identifier, descriptor)
                // Android 14 chooses layouts per input method and language,
                // so set it for every enabled one, whichever is in use.
                perInputMethod != null -> {
                    for ((ime, subtype) in inputMethodsAndSubtypes()) {
                        perInputMethod.invoke(im, identifier, 0, ime, subtype, descriptor)
                    }
                }
                // Earlier versions: a layout must be enabled for the keyboard
                // before it can be made the current one. Both are remembered
                // across restarts.
                setCurrent != null -> {
                    add?.invoke(im, identifier, descriptor)
                    setCurrent.invoke(im, identifier, descriptor)
                }
                else -> {
                    println("This Android version has no way to set a keyboard layout that AAE knows.")
                    exitProcess(2)
                }
            }
            println("Keyboard layout set for ${device.name}.")
            changed++
        }
        if (changed == 0) {
            println("No keyboard was found.")
            exitProcess(1)
        }
    }

    /**
     * Every enabled input method, paired with each of its enabled subtypes
     * (languages), or with null when it has none.
     */
    private fun inputMethodsAndSubtypes(): List<Pair<Any, Any?>> {
        val binder = Class.forName("android.os.ServiceManager")
            .getMethod("getService", String::class.java)
            .invoke(null, "input_method")
        val imm = Class.forName("com.android.internal.view.IInputMethodManager\$Stub")
            .getMethod("asInterface", Class.forName("android.os.IBinder"))
            .invoke(null, binder)!!
        val type = imm.javaClass
        val listMethods = type.methods.first { it.name == "getEnabledInputMethodList" }
        val imes = asList(listMethods.invoke(imm, 0))
        val subtypesMethod = type.methods.firstOrNull { it.name == "getEnabledInputMethodSubtypeList" }
        val pairs = mutableListOf<Pair<Any, Any?>>()
        for (ime in imes.filterNotNull()) {
            val id = ime.javaClass.getMethod("getId").invoke(ime) as String
            val subtypes = subtypesMethod?.let {
                when (it.parameterCount) {
                    3 -> it.invoke(imm, id, true, 0)
                    else -> it.invoke(imm, id, true)
                } as List<*>
            }.orEmpty().filterNotNull()
            if (subtypes.isEmpty()) {
                pairs += ime to null
            } else {
                subtypes.forEach { pairs += ime to it }
            }
        }
        return pairs
    }

    /**
     * A list of input methods as Android returns it: a plain list, or from
     * Android 15, an InputMethodInfoSafeList, which its static extractFrom
     * turns into one.
     */
    private fun asList(result: Any?): List<*> {
        if (result is List<*>) return result
        val type = result?.javaClass ?: return emptyList<Any>()
        val extract = type.methods.firstOrNull {
            java.lang.reflect.Modifier.isStatic(it.modifiers) &&
                it.parameterCount == 1 &&
                it.parameterTypes[0] == type &&
                List::class.java.isAssignableFrom(it.returnType)
        }
        if (extract != null) return extract.invoke(null, result) as List<*>
        val getter = type.methods.firstOrNull {
            !java.lang.reflect.Modifier.isStatic(it.modifiers) &&
                it.parameterCount == 0 &&
                List::class.java.isAssignableFrom(it.returnType)
        } ?: error("Android returned the input methods as ${type.name}, which AAE can't read")
        return getter.invoke(result) as List<*>
    }

    /** Physical full keyboards, with their input device identifiers. */
    private fun keyboards(): List<Pair<InputDevice, Any>> {
        val im = inputManager()
        val type = im.javaClass
        val ids = type.getMethod("getInputDeviceIds").invoke(im) as IntArray
        return ids.toList().mapNotNull { id ->
            val device = type.getMethod("getInputDevice", Int::class.javaPrimitiveType).invoke(im, id)
                as? InputDevice ?: return@mapNotNull null
            if (device.isVirtual || device.keyboardType != InputDevice.KEYBOARD_TYPE_ALPHABETIC) return@mapNotNull null
            device to InputDevice::class.java.getMethod("getIdentifier").invoke(device)!!
        }
    }

    /** Stops using the keyboard layout [descriptor], going back to the keyboard's own. */
    private fun clearKeyboardLayout(descriptor: String) {
        val im = inputManager()
        val type = im.javaClass
        for ((device, identifier) in keyboards()) {
            val override = type.methods.firstOrNull { it.name == "setKeyboardLayoutOverrideForInputDevice" }
            val remove = type.methods.firstOrNull { it.name == "removeKeyboardLayoutForInputDevice" }
            when {
                override != null -> override.invoke(im, identifier, null)
                remove != null -> remove.invoke(im, identifier, descriptor)
            }
            println("Keyboard layout cleared for ${device.name}.")
        }
    }

    /** Prints each keyboard's current and enabled layouts. */
    private fun describeKeyboards() {
        val im = inputManager()
        val type = im.javaClass
        for ((device, identifier) in keyboards()) {
            println("${device.name}: $identifier")
            type.methods.firstOrNull { it.name == "getCurrentKeyboardLayoutForInputDevice" }?.let {
                println("  current: ${it.invoke(im, identifier)}")
            }
            type.methods.firstOrNull { it.name == "getEnabledKeyboardLayoutsForInputDevice" }?.let {
                println("  enabled: ${(it.invoke(im, identifier) as Array<*>).joinToString()}")
            }
            type.methods.firstOrNull { it.name == "getKeyboardLayoutForInputDevice" && it.parameterCount == 4 }?.let {
                runCatching {
                    for ((ime, subtype) in inputMethodsAndSubtypes()) {
                        println("  for $ime / $subtype: ${it.invoke(im, identifier, 0, ime, subtype)}")
                    }
                }.onFailure { e -> println("  per input method: unavailable ($e)") }
            }
        }
    }

    private fun listInputManagerMethods() {
        for (method in inputManager().javaClass.methods.sortedBy { it.name }) {
            if ("Layout" in method.name || method.name == "getInputDevice" || method.name == "getInputDeviceIds") {
                println("${method.name}(${method.parameterTypes.joinToString { it.simpleName }}): ${method.returnType.simpleName}")
            }
        }
    }
}
