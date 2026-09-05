import React from "react";
import {
  ActivityIndicator,
  Pressable,
  StyleSheet,
  Text,
  TextInput,
  View,
  type PressableProps,
  type TextInputProps,
  type TextStyle,
  type ViewStyle,
} from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import * as Haptics from "expo-haptics";
import { radius, space, useTheme } from "./theme";

/** Full-screen container with safe-area insets and the theme background. */
export function Screen({
  children,
  style,
  padded = true,
}: {
  children: React.ReactNode;
  style?: ViewStyle;
  padded?: boolean;
}) {
  const t = useTheme();
  return (
    <SafeAreaView style={[{ flex: 1, backgroundColor: t.bg }, style]}>
      <View style={{ flex: 1, paddingHorizontal: padded ? space.xl : 0 }}>{children}</View>
    </SafeAreaView>
  );
}

export function Title({ children, style }: { children: React.ReactNode; style?: TextStyle }) {
  const t = useTheme();
  return (
    <Text style={[{ fontSize: 28, fontWeight: "700", letterSpacing: -0.5, color: t.text }, style]}>
      {children}
    </Text>
  );
}

export function Body({ children, style, muted }: { children: React.ReactNode; style?: TextStyle; muted?: boolean }) {
  const t = useTheme();
  return (
    <Text style={[{ fontSize: 16, lineHeight: 22, color: muted ? t.muted : t.text }, style]}>
      {children}
    </Text>
  );
}

export function Caption({ children, style }: { children: React.ReactNode; style?: TextStyle }) {
  const t = useTheme();
  return <Text style={[{ fontSize: 13, lineHeight: 18, color: t.muted }, style]}>{children}</Text>;
}

type ButtonProps = PressableProps & {
  title: string;
  variant?: "primary" | "secondary" | "ghost" | "danger";
  loading?: boolean;
  icon?: React.ReactNode;
};

export function Button({ title, variant = "primary", loading, icon, disabled, onPress, style, ...rest }: ButtonProps) {
  const t = useTheme();
  const bg =
    variant === "primary" ? t.brand : variant === "secondary" ? t.surface : "transparent";
  const fg =
    variant === "primary" ? t.onBrand : variant === "danger" ? t.danger : t.text;
  return (
    <Pressable
      accessibilityRole="button"
      disabled={disabled || loading}
      onPress={(e) => {
        void Haptics.selectionAsync();
        onPress?.(e);
      }}
      style={({ pressed }) => [
        styles.button,
        { backgroundColor: bg, opacity: disabled ? 0.5 : pressed ? 0.85 : 1 },
        variant === "secondary" && { borderWidth: 1, borderColor: t.border },
        style as ViewStyle,
      ]}
      {...rest}
    >
      {loading ? (
        <ActivityIndicator color={fg} />
      ) : (
        <View style={{ flexDirection: "row", alignItems: "center", gap: space.sm }}>
          {icon}
          <Text style={{ color: fg, fontSize: 16, fontWeight: "600" }}>{title}</Text>
        </View>
      )}
    </Pressable>
  );
}

export const Field = React.forwardRef<TextInput, TextInputProps>(function Field(props, ref) {
  const t = useTheme();
  return (
    <TextInput
      ref={ref}
      placeholderTextColor={t.muted}
      {...props}
      style={[
        styles.field,
        { backgroundColor: t.surface, borderColor: t.border, color: t.text },
        props.style,
      ]}
    />
  );
});

export function Card({ children, style }: { children: React.ReactNode; style?: ViewStyle }) {
  const t = useTheme();
  return (
    <View style={[styles.card, { backgroundColor: t.surface, borderColor: t.border }, style]}>
      {children}
    </View>
  );
}

/** A settings-style row: label on the left, optional value or chevron on the right. */
export function Row({
  label,
  value,
  onPress,
  danger,
}: {
  label: string;
  value?: string;
  onPress?: () => void;
  danger?: boolean;
}) {
  const t = useTheme();
  return (
    <Pressable
      accessibilityRole={onPress ? "button" : undefined}
      onPress={onPress}
      disabled={!onPress}
      style={({ pressed }) => [styles.row, { borderColor: t.border, opacity: pressed ? 0.7 : 1 }]}
    >
      <Text style={{ fontSize: 16, color: danger ? t.danger : t.text }}>{label}</Text>
      <Text style={{ fontSize: 15, color: t.muted }}>{value ?? (onPress ? "›" : "")}</Text>
    </Pressable>
  );
}

export function Spacer({ h = space.lg }: { h?: number }) {
  return <View style={{ height: h }} />;
}

export function Centered({ children }: { children: React.ReactNode }) {
  return <View style={{ flex: 1, alignItems: "center", justifyContent: "center", gap: space.md }}>{children}</View>;
}

const styles = StyleSheet.create({
  button: {
    minHeight: 52,
    paddingHorizontal: space.xl,
    borderRadius: radius.md,
    alignItems: "center",
    justifyContent: "center",
  },
  field: {
    minHeight: 52,
    borderWidth: 1,
    borderRadius: radius.md,
    paddingHorizontal: space.lg,
    fontSize: 16,
  },
  card: {
    borderWidth: 1,
    borderRadius: radius.lg,
    padding: space.lg,
    gap: space.sm,
  },
  row: {
    minHeight: 52,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
});
