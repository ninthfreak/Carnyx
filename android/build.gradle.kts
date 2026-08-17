// AGP 8.7.3 against Gradle 8.14.3 and JDK 17+.
//
// Pinned rather than floating: an Android build that silently changes its
// packaging rules between machines is exactly the class of surprise this move
// was meant to end.
plugins {
    id("com.android.application") version "8.7.3" apply false
}
