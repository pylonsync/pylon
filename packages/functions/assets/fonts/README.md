# Bundled fonts

`Inter-Regular.ttf` (weight 400) and `Inter-SemiBold.ttf` (weight 600) are the
default fonts for dynamic OpenGraph image rendering (`opengraph-image.tsx` →
Satori). Satori requires static (non-variable) TTF/OTF; these are the static
Inter cuts. Inter is licensed under the SIL Open Font License 1.1
(https://github.com/rsms/inter). Apps can supply their own fonts via
`ImageResponse`'s `fonts` option.
