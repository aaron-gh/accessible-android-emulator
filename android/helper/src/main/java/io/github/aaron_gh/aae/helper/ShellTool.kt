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
        when (args.firstOrNull()) {
            "methods" -> listInputManagerMethods()
            "layouts" -> listKeyboardLayouts()
            "keyboard-layout" -> useKeyboardLayout(args.getOrElse(1) { KeyboardLayouts.FULL_KEYBOARD })
            else -> println("Usage: ShellTool methods | layouts | keyboard-layout [descriptor]")
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
            val setter = type.methods.firstOrNull { it.name == "setKeyboardLayoutOverrideForInputDevice" }
                ?: type.methods.firstOrNull { it.name == "setCurrentKeyboardLayoutForInputDevice" }
            if (setter == null) {
                println("This Android version has no way to set a keyboard layout that AAE knows.")
                exitProcess(2)
            }
            setter.invoke(im, identifier, descriptor)
            println("Keyboard layout set for ${device.name}.")
            changed++
        }
        if (changed == 0) {
            println("No keyboard was found.")
            exitProcess(1)
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
