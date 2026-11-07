//! The Classic -> Kingdom Reborn land mapping the KR facet encoder needs: for each Classic land id, the KR
//! material (`TerrainDefinition` id) and the `subtype` (alias of the material) a KR square stores with it.
//!
//! Decoding needs no mapping: KR squares store their Classic id ([`KrSquareLand::classic_land`]).
//!
//! The bundled `assets/kr/mappings.bin` is generated from the shipped KR facets (`UO - Old KR`, facets 0-4, 69,746,240 squares):
//! for each Classic id, the most frequent `material` and the most frequent `subtype` (ties to the smaller value).
//! It is exact for `material` on 99.9995% of the shipped squares (only Classic 2444 and 2446 take two KR ids) and
//! for `subtype` on 99.967% (64 Classic ids take more than one value); the ignored test
//! `kr_facet_sectors_reencode_identically_and_follow_the_rules` regenerates and checks it. [`BUILDER_KR_LAND_MAP_SUPPLEMENT`] adds the
//! Classic ids the KR facets never use, from Manawydan's KR converter table (`tile_mappings_builder`, feature
//! `tile_mappings_builder`); they are unverified.

use std::collections::{BTreeMap, HashMap};

use crate::enhanced::facet_decoder::{DecodedEcFacet, KrSquareLand};

crate::eyre_imports!();

const SHIPPED_KR_LAND_BINARY: &[u8] = include_bytes!("../../../assets/kr/mappings.bin");

/// `(classic land id, material, subtype)` of the Classic ids the shipped KR facets do not use, from the
/// `tile_mappings_builder` table (unverified), by Classic id.
pub const BUILDER_KR_LAND_MAP_SUPPLEMENT: [(u16, u16, u8); 1133] = [
    (7, 1, 0), (8, 1, 0), (44, 54, 0), (63, 7, 0), (64, 7, 0), (65, 7, 0), (66, 56, 0), (67, 56, 0),
    (71, 54, 0), (103, 52, 0), (112, 52, 0), (177, 3, 0), (178, 3, 0), (181, 3, 0), (184, 3, 0), (186, 3, 0),
    (187, 3, 0), (267, 3, 0), (280, 8, 0), (302, 7, 0), (303, 7, 0), (304, 7, 0), (305, 7, 0), (306, 5, 0),
    (307, 5, 0), (308, 5, 0), (309, 5, 0), (312, 5, 0), (313, 5, 0), (314, 5, 0), (315, 5, 0), (316, 5, 0),
    (317, 2, 0), (318, 2, 0), (319, 2, 0), (320, 2, 0), (325, 2, 0), (326, 2, 0), (327, 0, 0), (330, 0, 0),
    (346, 21, 0), (347, 21, 0), (348, 4, 0), (373, 2, 0), (374, 2, 0), (375, 2, 0), (376, 2, 0), (395, 61, 1),
    (396, 61, 1), (397, 61, 1), (398, 61, 1), (399, 61, 1), (400, 61, 1), (401, 61, 1), (402, 61, 1), (403, 61, 1),
    (404, 61, 1), (405, 61, 1), (406, 61, 1), (407, 61, 1), (408, 1, 0), (409, 1, 0), (410, 1, 0), (411, 1, 0),
    (412, 1, 0), (413, 1, 0), (414, 1, 0), (415, 1, 0), (416, 1, 0), (417, 1, 0), (418, 1, 0), (419, 1, 0),
    (422, 1, 0), (428, 7, 0), (429, 7, 0), (433, 0, 0), (434, 0, 0), (435, 0, 0), (436, 0, 0), (437, 54, 0),
    (438, 54, 0), (439, 54, 0), (440, 54, 0), (464, 55, 0), (466, 54, 0), (488, 2, 0), (489, 2, 0), (490, 2, 0),
    (491, 2, 0), (498, 3, 0), (499, 3, 0), (504, 10, 0), (505, 10, 0), (654, 7, 0), (655, 7, 0), (656, 7, 0),
    (657, 7, 0), (658, 7, 0), (659, 7, 0), (660, 7, 0), (661, 7, 0), (703, 15, 1), (706, 16, 1), (707, 14, 1),
    (710, 14, 1), (713, 16, 1), (716, 16, 1), (717, 16, 1), (718, 56, 0), (719, 56, 0), (721, 56, 0), (723, 54, 0),
    (725, 56, 0), (727, 54, 0), (728, 54, 0), (729, 54, 0), (730, 54, 0), (731, 54, 0), (732, 54, 0), (733, 54, 0),
    (734, 58, 0), (735, 58, 0), (736, 58, 0), (737, 58, 0), (738, 58, 0), (739, 58, 0), (740, 58, 0), (768, 58, 0),
    (769, 58, 0), (770, 58, 0), (780, 63, 1), (781, 63, 1), (782, 63, 1), (787, 63, 1), (788, 63, 1), (789, 63, 1),
    (791, 7, 0), (792, 7, 0), (793, 7, 0), (794, 7, 0), (795, 7, 0), (796, 7, 0), (797, 7, 0), (798, 7, 0),
    (799, 7, 0), (800, 7, 0), (801, 7, 0), (802, 7, 0), (803, 7, 0), (808, 7, 0), (810, 7, 0), (812, 7, 0),
    (813, 7, 0), (814, 7, 0), (815, 7, 0), (816, 7, 0), (817, 7, 0), (818, 7, 0), (819, 7, 0), (820, 7, 0),
    (833, 2, 0), (842, 2, 0), (861, 7, 0), (862, 7, 0), (863, 7, 0), (864, 7, 0), (865, 7, 0), (866, 2, 0),
    (867, 2, 0), (868, 2, 0), (869, 2, 0), (870, 2, 0), (895, 1, 0), (896, 1, 0), (897, 1, 0), (898, 8, 1),
    (899, 8, 1), (900, 8, 1), (915, 8, 1), (926, 2, 0), (927, 8, 1), (932, 8, 1), (941, 8, 1), (942, 8, 1),
    (943, 8, 1), (944, 8, 1), (945, 8, 1), (946, 8, 1), (947, 7, 0), (948, 7, 0), (949, 7, 0), (950, 7, 0),
    (951, 7, 0), (952, 7, 0), (954, 7, 0), (959, 7, 0), (962, 1, 0), (975, 1, 0), (976, 1, 0), (977, 1, 0),
    (978, 1, 0), (979, 1, 0), (980, 1, 0), (981, 1, 0), (982, 1, 0), (983, 1, 0), (984, 1, 0), (985, 1, 0),
    (986, 1, 0), (987, 1, 0), (988, 1, 0), (989, 1, 0), (990, 1, 0), (991, 1, 0), (992, 1, 0), (993, 1, 0),
    (994, 1, 0), (995, 1, 0), (996, 1, 0), (997, 1, 0), (998, 1, 0), (999, 1, 0), (1000, 1, 0), (1005, 9, 0),
    (1009, 0, 0), (1013, 0, 0), (1131, 30, 0), (1175, 39, 0), (1176, 39, 0), (1177, 39, 0), (1178, 39, 0), (1179, 39, 0),
    (1180, 39, 0), (1181, 39, 0), (1182, 39, 0), (1183, 39, 0), (1184, 39, 0), (1185, 39, 0), (1186, 39, 0), (1187, 39, 0),
    (1188, 39, 0), (1189, 39, 0), (1190, 39, 0), (1191, 39, 0), (1192, 39, 0), (1193, 39, 0), (1194, 39, 0), (1195, 39, 0),
    (1196, 39, 0), (1197, 39, 0), (1198, 39, 0), (1199, 39, 0), (1200, 39, 0), (1201, 39, 0), (1202, 39, 0), (1223, 128, 0),
    (1224, 128, 0), (1225, 128, 0), (1226, 0, 0), (1227, 0, 0), (1228, 0, 0), (1230, 0, 0), (1231, 0, 0), (1232, 0, 0),
    (1233, 0, 0), (1234, 0, 0), (1235, 0, 0), (1236, 0, 0), (1237, 0, 0), (1238, 0, 0), (1239, 0, 0), (1240, 0, 0),
    (1241, 0, 0), (1242, 0, 0), (1243, 0, 0), (1244, 0, 0), (1245, 0, 0), (1246, 0, 0), (1247, 0, 0), (1248, 0, 0),
    (1249, 0, 0), (1250, 0, 0), (1251, 0, 0), (1252, 0, 0), (1253, 0, 0), (1254, 0, 0), (1267, 43, 0), (1268, 43, 0),
    (1269, 43, 0), (1270, 43, 0), (1271, 43, 0), (1272, 43, 0), (1273, 43, 0), (1274, 43, 0), (1275, 43, 0), (1276, 43, 0),
    (1277, 43, 0), (1278, 43, 0), (1279, 43, 0), (1280, 43, 0), (1300, 2, 0), (1301, 2, 0), (1302, 2, 0), (1303, 2, 0),
    (1304, 2, 0), (1305, 2, 0), (1306, 2, 0), (1307, 2, 0), (1308, 2, 0), (1309, 2, 0), (1310, 2, 0), (1311, 2, 0),
    (1312, 2, 0), (1313, 2, 0), (1314, 2, 0), (1315, 2, 0), (1316, 2, 0), (1317, 2, 0), (1318, 2, 0), (1319, 2, 0),
    (1320, 2, 0), (1321, 2, 0), (1322, 2, 0), (1323, 2, 0), (1324, 2, 0), (1325, 2, 0), (1326, 2, 0), (1327, 2, 0),
    (1328, 2, 0), (1329, 2, 0), (1330, 2, 0), (1331, 2, 0), (1332, 2, 0), (1333, 2, 0), (1334, 2, 0), (1335, 2, 0),
    (1336, 2, 0), (1337, 2, 0), (1338, 2, 0), (1339, 2, 0), (1340, 2, 0), (1341, 2, 0), (1342, 2, 0), (1343, 2, 0),
    (1344, 2, 0), (1345, 2, 0), (1346, 2, 0), (1347, 2, 0), (1348, 2, 0), (1349, 2, 0), (1350, 2, 0), (1367, 2, 0),
    (1368, 2, 0), (1369, 2, 0), (1370, 2, 0), (1371, 2, 0), (1372, 2, 0), (1373, 2, 0), (1374, 2, 0), (1375, 2, 0),
    (1376, 2, 0), (1377, 2, 0), (1378, 2, 0), (1379, 2, 0), (1380, 2, 0), (1381, 2, 0), (1382, 2, 0), (1383, 2, 0),
    (1384, 2, 0), (1385, 1, 0), (1386, 1, 0), (1387, 1, 0), (1388, 1, 0), (1389, 1, 0), (1390, 1, 0), (1391, 1, 0),
    (1392, 1, 0), (1393, 1, 0), (1394, 1, 0), (1395, 1, 0), (1396, 1, 0), (1397, 1, 0), (1398, 1, 0), (1399, 1, 0),
    (1400, 1, 0), (1401, 1, 0), (1404, 1, 0), (1408, 1, 0), (1414, 3, 0), (1425, 3, 0), (1426, 3, 0), (1427, 3, 0),
    (1428, 2, 0), (1429, 2, 0), (1430, 2, 0), (1432, 2, 0), (1437, 2, 0), (1440, 3, 0), (1443, 3, 0), (1444, 3, 0),
    (1445, 3, 0), (1447, 3, 0), (1448, 3, 0), (1449, 3, 0), (1450, 3, 0), (1451, 3, 0), (1452, 3, 0), (1453, 3, 0),
    (1454, 3, 0), (1455, 3, 0), (1456, 3, 0), (1457, 3, 0), (1458, 3, 0), (1459, 3, 0), (1460, 3, 0), (1461, 3, 0),
    (1462, 3, 0), (1463, 3, 0), (1464, 3, 0), (1465, 3, 0), (1466, 3, 0), (1467, 3, 0), (1468, 3, 0), (1469, 3, 0),
    (1470, 3, 0), (1471, 3, 0), (1472, 3, 0), (1473, 3, 0), (1474, 3, 0), (1475, 3, 0), (1476, 3, 0), (1477, 3, 0),
    (1478, 3, 0), (1479, 3, 0), (1480, 3, 0), (1481, 3, 0), (1482, 3, 0), (1483, 3, 0), (1484, 3, 0), (1485, 4, 0),
    (1486, 4, 0), (1487, 4, 0), (1488, 4, 0), (1489, 4, 0), (1490, 4, 0), (1491, 4, 0), (1492, 4, 0), (1493, 4, 0),
    (1494, 4, 0), (1495, 4, 0), (1496, 4, 0), (1497, 4, 0), (1498, 4, 0), (1499, 4, 0), (1500, 4, 0), (1501, 4, 0),
    (1502, 4, 0), (1503, 4, 0), (1504, 4, 0), (1505, 4, 0), (1506, 4, 0), (1507, 4, 0), (1508, 4, 0), (1509, 4, 0),
    (1510, 4, 0), (1511, 4, 0), (1512, 4, 0), (1513, 4, 0), (1514, 4, 0), (1515, 4, 0), (1516, 4, 0), (1517, 4, 0),
    (1518, 4, 0), (1519, 4, 0), (1520, 4, 0), (1521, 4, 0), (1527, 3, 0), (1529, 3, 0), (1532, 4, 0), (1533, 4, 0),
    (1534, 4, 0), (1535, 4, 0), (1536, 4, 0), (1537, 4, 0), (1538, 4, 0), (1539, 3, 0), (1540, 3, 0), (1541, 3, 0),
    (1542, 3, 0), (1543, 3, 0), (1544, 3, 0), (1545, 3, 0), (1547, 3, 0), (1550, 3, 0), (1551, 3, 0), (1552, 4, 0),
    (1554, 4, 0), (1556, 3, 0), (1559, 3, 0), (1561, 3, 0), (1562, 3, 0), (1563, 3, 0), (1564, 3, 0), (1565, 3, 0),
    (1566, 2, 0), (1567, 2, 0), (1568, 2, 0), (1569, 2, 0), (1570, 2, 0), (1582, 2, 0), (1599, 13, 0), (1600, 13, 0),
    (1601, 13, 0), (1602, 13, 0), (1603, 13, 0), (1604, 13, 0), (1605, 7, 0), (1606, 7, 0), (1607, 7, 0), (1608, 7, 0),
    (1609, 7, 0), (1610, 7, 0), (1614, 7, 0), (1615, 7, 0), (1617, 7, 0), (1620, 4, 0), (1622, 4, 0), (1623, 4, 0),
    (1624, 7, 0), (1626, 7, 0), (1627, 7, 0), (1628, 7, 0), (1629, 7, 0), (1630, 7, 0), (1631, 7, 0), (1632, 7, 0),
    (1633, 7, 0), (1634, 7, 0), (1635, 7, 0), (1636, 7, 0), (1637, 7, 0), (1638, 7, 0), (1639, 7, 0), (1640, 7, 0),
    (1641, 7, 0), (1642, 7, 0), (1643, 7, 0), (1644, 7, 0), (1645, 1, 0), (1646, 1, 0), (1647, 1, 0), (1648, 1, 0),
    (1649, 1, 0), (1650, 1, 0), (1651, 1, 0), (1652, 1, 0), (1653, 1, 0), (1654, 1, 0), (1655, 1, 0), (1656, 1, 0),
    (1657, 1, 0), (1658, 1, 0), (1659, 1, 0), (1660, 1, 0), (1675, 9, 0), (1683, 9, 0), (1685, 9, 0), (1698, 4, 0),
    (1699, 4, 0), (1700, 4, 0), (1701, 4, 0), (1702, 4, 0), (1703, 4, 0), (1704, 4, 0), (1705, 4, 0), (1706, 4, 0),
    (1707, 4, 0), (1708, 4, 0), (1709, 4, 0), (1710, 4, 0), (1731, 1, 0), (1732, 1, 0), (1733, 1, 0), (1734, 1, 0),
    (1735, 1, 0), (1736, 6, 0), (1737, 6, 0), (1738, 6, 0), (1739, 6, 0), (1740, 6, 0), (1762, 1, 0), (1763, 1, 0),
    (1764, 1, 0), (1765, 1, 0), (1766, 1, 0), (1767, 6, 0), (1768, 6, 0), (1769, 6, 0), (1770, 6, 0), (1776, 6, 0),
    (1791, 6, 0), (1792, 6, 0), (1793, 6, 0), (1794, 6, 0), (1795, 6, 0), (1796, 4, 0), (1797, 4, 0), (1798, 4, 0),
    (1799, 4, 0), (1800, 4, 0), (1805, 4, 0), (1806, 6, 0), (1808, 6, 0), (1809, 6, 0), (1810, 6, 0), (1825, 6, 0),
    (1826, 6, 0), (1827, 6, 0), (1828, 6, 0), (1829, 6, 0), (1830, 6, 0), (1831, 6, 0), (1832, 6, 0), (1833, 6, 0),
    (1834, 6, 0), (1835, 6, 0), (1836, 6, 0), (1837, 6, 0), (1838, 6, 0), (1839, 6, 0), (1840, 6, 0), (1841, 6, 0),
    (1842, 6, 0), (1843, 6, 0), (1844, 6, 0), (1845, 6, 0), (1846, 6, 0), (1847, 6, 0), (1848, 6, 0), (1849, 6, 0),
    (1850, 6, 0), (1855, 6, 0), (1856, 6, 0), (1857, 6, 0), (1858, 8, 1), (1859, 8, 1), (1860, 8, 1), (1864, 8, 1),
    (1865, 6, 0), (1868, 6, 0), (1869, 6, 0), (1870, 8, 1), (1872, 8, 1), (1881, 8, 0), (1889, 8, 1), (1890, 8, 1),
    (1891, 8, 1), (1892, 8, 1), (1893, 8, 1), (1894, 8, 1), (1895, 63, 1), (1896, 63, 1), (1897, 63, 1), (1898, 63, 1),
    (1899, 63, 1), (1900, 63, 1), (1908, 61, 1), (1909, 61, 1), (1910, 58, 0), (1938, 58, 0), (1939, 58, 0), (1940, 58, 0),
    (1941, 58, 0), (1942, 2, 0), (1943, 2, 0), (1944, 2, 0), (1945, 2, 0), (1966, 9, 0), (1970, 2, 0), (1971, 2, 0),
    (1972, 2, 0), (1973, 2, 0), (1974, 2, 0), (1975, 2, 0), (1976, 7, 0), (1977, 7, 0), (1978, 7, 0), (1979, 7, 0),
    (1980, 7, 0), (1990, 6, 0), (2005, 6, 1), (2006, 6, 1), (2007, 6, 1), (2008, 6, 1), (2009, 6, 1), (2010, 6, 1),
    (2011, 6, 1), (2012, 6, 1), (2013, 6, 1), (2014, 6, 1), (2015, 6, 1), (2016, 6, 1), (2017, 6, 1), (2018, 6, 1),
    (2019, 6, 1), (2020, 6, 1), (2021, 6, 1), (2022, 6, 1), (2023, 6, 1), (2024, 6, 1), (2025, 6, 1), (2026, 6, 1),
    (2027, 6, 1), (2034, 44, 0), (2035, 44, 0), (2036, 44, 0), (2037, 44, 0), (2038, 44, 0), (2039, 44, 0), (2040, 44, 0),
    (2041, 44, 0), (2042, 44, 0), (2043, 44, 0), (2044, 44, 0), (2045, 44, 0), (2046, 44, 0), (2047, 44, 0), (2048, 44, 0),
    (2049, 44, 0), (2050, 44, 0), (2051, 44, 0), (2052, 44, 0), (2053, 44, 0), (2054, 44, 0), (2055, 44, 0), (2056, 44, 0),
    (2057, 44, 0), (2058, 44, 0), (2059, 44, 0), (2060, 44, 0), (2061, 44, 0), (2062, 44, 0), (2063, 44, 0), (2064, 44, 0),
    (2065, 44, 0), (2066, 44, 0), (2067, 44, 0), (2068, 44, 0), (2069, 44, 0), (2070, 44, 0), (2071, 44, 0), (2072, 44, 0),
    (2073, 44, 0), (2074, 44, 0), (2075, 44, 0), (2076, 44, 0), (2077, 44, 0), (2078, 44, 0), (2079, 44, 0), (2080, 44, 0),
    (2081, 44, 0), (2082, 44, 0), (2083, 44, 0), (2084, 44, 0), (2085, 44, 0), (2086, 44, 0), (2087, 44, 0), (2088, 44, 0),
    (2089, 44, 0), (2090, 44, 0), (2091, 44, 0), (2092, 44, 0), (2093, 44, 0), (2094, 44, 0), (2095, 44, 0), (2096, 44, 0),
    (2097, 44, 0), (2098, 44, 0), (2099, 44, 0), (2106, 45, 0), (2107, 45, 0), (2108, 45, 0), (2109, 45, 0), (2110, 45, 0),
    (2111, 45, 0), (2112, 45, 0), (2113, 45, 0), (2114, 45, 0), (2115, 45, 0), (2116, 45, 0), (2117, 45, 0), (2118, 45, 0),
    (2119, 45, 0), (2120, 45, 0), (2121, 45, 0), (2122, 45, 0), (2123, 45, 0), (2124, 45, 0), (2125, 45, 0), (2126, 45, 0),
    (2127, 45, 0), (2128, 45, 0), (2129, 45, 0), (2130, 45, 0), (2131, 45, 0), (2132, 45, 0), (2133, 45, 0), (2134, 45, 0),
    (2135, 45, 0), (2136, 45, 0), (2137, 45, 0), (2138, 45, 0), (2139, 45, 0), (2140, 45, 0), (2141, 45, 0), (2142, 45, 0),
    (2143, 45, 0), (2144, 45, 0), (2145, 45, 0), (2146, 45, 0), (2147, 45, 0), (2148, 45, 0), (2149, 45, 0), (2150, 45, 0),
    (2151, 45, 0), (2152, 45, 0), (2153, 45, 0), (2154, 45, 0), (2155, 45, 0), (2156, 45, 0), (2157, 45, 0), (2158, 45, 0),
    (2159, 45, 0), (2160, 45, 0), (2161, 45, 0), (2162, 45, 0), (2163, 45, 0), (2164, 45, 0), (2165, 45, 0), (2166, 45, 0),
    (2167, 45, 0), (2168, 45, 0), (2169, 45, 0), (2170, 45, 0), (2171, 45, 0), (2172, 45, 0), (2173, 45, 0), (2174, 45, 0),
    (2175, 45, 0), (2176, 45, 0), (2177, 45, 0), (2178, 45, 0), (2179, 45, 0), (2180, 45, 0), (2181, 45, 0), (2182, 45, 0),
    (2183, 45, 0), (2184, 45, 0), (2185, 45, 0), (2186, 45, 0), (2187, 45, 0), (2188, 45, 0), (2189, 45, 0), (2190, 45, 0),
    (2191, 45, 0), (2192, 45, 0), (2193, 45, 0), (2194, 45, 0), (2195, 45, 0), (2196, 45, 0), (2197, 45, 0), (2198, 45, 0),
    (2199, 45, 0), (2200, 45, 0), (2201, 45, 0), (2202, 45, 0), (2203, 45, 0), (2204, 45, 0), (2205, 45, 0), (2206, 45, 0),
    (2207, 45, 0), (2208, 45, 0), (2209, 45, 0), (2210, 45, 0), (2211, 45, 0), (2212, 45, 0), (2213, 45, 0), (2214, 45, 0),
    (2215, 45, 0), (2216, 45, 0), (2217, 45, 0), (2218, 45, 0), (2219, 45, 0), (2220, 45, 0), (2221, 45, 0), (2222, 45, 0),
    (2223, 45, 0), (2224, 45, 0), (2225, 45, 0), (2226, 45, 0), (2227, 45, 0), (2228, 45, 0), (2229, 45, 0), (2230, 45, 0),
    (2231, 45, 0), (2232, 45, 0), (2233, 45, 0), (2234, 45, 0), (2235, 45, 0), (2236, 45, 0), (2237, 45, 0), (2238, 45, 0),
    (2239, 45, 0), (2240, 45, 0), (2241, 45, 0), (2242, 45, 0), (2243, 45, 0), (2244, 45, 0), (2245, 45, 0), (2246, 45, 0),
    (2247, 45, 0), (2248, 45, 0), (2249, 45, 0), (2250, 45, 0), (2251, 45, 0), (2252, 45, 0), (2253, 45, 0), (2254, 45, 0),
    (2255, 45, 0), (2256, 45, 0), (2257, 45, 0), (2258, 45, 0), (2259, 45, 0), (2260, 45, 0), (2261, 45, 0), (2262, 45, 0),
    (2263, 45, 0), (2264, 45, 0), (2265, 45, 0), (2266, 45, 0), (2267, 45, 0), (2268, 45, 0), (2269, 45, 0), (2270, 45, 0),
    (2271, 45, 0), (2272, 45, 0), (2273, 45, 0), (2274, 45, 0), (2275, 45, 0), (2276, 45, 0), (2277, 45, 0), (2278, 45, 0),
    (2279, 45, 0), (2280, 45, 0), (2281, 45, 0), (2282, 45, 0), (2283, 45, 0), (2284, 45, 0), (2285, 45, 0), (2286, 45, 0),
    (2287, 45, 0), (2288, 45, 0), (2289, 45, 0), (2290, 45, 0), (2291, 45, 0), (2292, 45, 0), (2293, 45, 0), (2294, 45, 0),
    (2295, 45, 0), (2296, 45, 0), (2297, 45, 0), (2298, 45, 0), (2299, 45, 0), (2300, 45, 0), (2301, 45, 0), (2302, 45, 0),
    (2303, 45, 0), (2304, 45, 0), (2305, 45, 0), (2306, 45, 0), (2307, 45, 0), (2308, 45, 0), (2309, 45, 0), (2310, 45, 0),
    (2311, 45, 0), (2312, 45, 0), (2313, 45, 0), (2314, 45, 0), (2315, 45, 0), (2316, 45, 0), (2317, 45, 0), (2318, 45, 0),
    (2319, 45, 0), (2320, 45, 0), (2321, 45, 0), (2322, 45, 0), (2323, 45, 0), (2324, 45, 0), (2325, 45, 0), (2326, 45, 0),
    (2327, 45, 0), (2328, 45, 0), (2329, 45, 0), (2330, 45, 0), (2331, 45, 0), (2332, 45, 0), (2333, 45, 0), (2334, 45, 0),
    (2335, 45, 0), (2336, 45, 0), (2337, 45, 0), (2338, 45, 0), (2339, 45, 0), (2340, 45, 0), (2341, 45, 0), (2342, 45, 0),
    (2343, 45, 0), (2344, 45, 0), (2345, 45, 0), (2346, 45, 0), (2347, 45, 0), (2348, 45, 0), (2349, 45, 0), (2350, 45, 0),
    (2351, 45, 0), (2352, 45, 0), (2353, 45, 0), (2354, 45, 0), (2355, 45, 0), (2356, 45, 0), (2357, 45, 0), (2358, 45, 0),
    (2359, 45, 0), (2360, 45, 0), (2361, 45, 0), (2362, 45, 0), (2363, 45, 0), (2364, 45, 0), (2365, 45, 0), (2366, 45, 0),
    (2367, 45, 0), (2368, 45, 0), (2369, 45, 0), (2370, 45, 0), (2371, 45, 0), (2372, 45, 0), (2373, 45, 0), (2374, 45, 0),
    (2375, 45, 0), (2376, 45, 0), (2377, 45, 0), (2378, 45, 0), (2379, 45, 0), (2380, 45, 0), (2381, 45, 0), (2382, 45, 0),
    (2383, 45, 0), (2384, 45, 0), (2385, 45, 0), (2386, 45, 0), (2387, 45, 0), (2388, 45, 0), (2389, 45, 0), (2390, 45, 0),
    (2391, 45, 0), (2392, 45, 0), (2393, 45, 0), (2394, 45, 0), (2395, 45, 0), (2396, 45, 0), (2397, 45, 0), (2398, 45, 0),
    (2399, 45, 0), (2400, 45, 0), (2401, 45, 0), (2402, 45, 0), (2403, 45, 0), (2404, 45, 0), (2405, 45, 0), (2406, 45, 0),
    (2407, 45, 0), (2408, 45, 0), (2409, 45, 0), (2410, 45, 0), (2411, 45, 0), (2412, 45, 0), (2413, 45, 0), (2414, 45, 0),
    (2415, 45, 0), (2416, 45, 0), (2417, 45, 0), (2418, 45, 0), (2419, 45, 0), (2420, 45, 0), (2421, 45, 0), (2422, 45, 0),
    (2423, 45, 0), (2424, 45, 0), (2425, 45, 0), (2426, 45, 0), (2427, 45, 0), (2428, 45, 0), (2429, 45, 0), (2430, 45, 0),
    (2431, 45, 0), (2432, 45, 0), (2433, 45, 0), (2434, 45, 0), (2435, 45, 0), (2436, 45, 0), (2437, 45, 0), (2438, 45, 0),
    (2439, 45, 0), (2440, 45, 0), (2441, 45, 0), (2442, 45, 0), (2443, 45, 0), (2464, 58, 0), (2465, 58, 0), (2466, 58, 0),
    (2467, 58, 0), (2468, 58, 0), (2469, 58, 0), (2470, 58, 0), (2471, 58, 0), (2472, 58, 0), (2473, 58, 0), (2474, 58, 0),
    (2475, 58, 0), (2496, 58, 0), (2497, 58, 0), (2498, 58, 0), (2499, 58, 0),
];

/// How a KR square stores one Classic land id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KrLand {
    pub material: u16,
    pub subtype: u8,
}

/// A Classic -> KR land mapping.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KrLandMap {
    map: HashMap<u16, KrLand>,
    /// What [`Self::square_land`] stores for a Classic id the map lacks; `None` fails instead.
    fallback: Option<KrLand>,
}

impl KrLandMap {
    /// The bundled verified binary plus [`BUILDER_KR_LAND_MAP_SUPPLEMENT`] (unverified).
    pub fn shipped() -> Self {
        let mut map = Self::shipped_verified();
        for &(classic, material, subtype) in &BUILDER_KR_LAND_MAP_SUPPLEMENT {
            map.map.entry(classic).or_insert(KrLand { material, subtype });
        }
        map
    }

    /// Only the ids verified against the shipped facets, read from the bundled KRLM binary.
    pub fn shipped_verified() -> Self {
        Self::from_binary(SHIPPED_KR_LAND_BINARY).expect("bundled KR land mapping must be valid KRLM")
    }

    /// Reads a KRLM v1 binary. Rejects unsupported headers, length mismatches,
    /// duplicates and unsorted ids. No fallback is stored in this format.
    pub fn from_binary(bytes: &[u8]) -> eyre::Result<Self> {
        const HEADER_SIZE: usize = 12;
        const RECORD_SIZE: usize = 5;
        eyre::ensure!(bytes.len() >= HEADER_SIZE, "KRLM header is truncated");
        eyre::ensure!(&bytes[..4] == b"KRLM", "invalid KRLM magic");
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        eyre::ensure!(version == 1, "unsupported KRLM version {version}");
        let record_size = u16::from_le_bytes([bytes[6], bytes[7]]);
        eyre::ensure!(usize::from(record_size) == RECORD_SIZE, "invalid KRLM record size {record_size}");
        let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        eyre::ensure!(count <= 65_536, "KRLM record count exceeds the Classic id range");
        eyre::ensure!(bytes.len() == HEADER_SIZE + count as usize * RECORD_SIZE, "KRLM record count/length mismatch");
        let mut map = Self::default();
        let mut previous = None;
        for row in bytes[HEADER_SIZE..].chunks_exact(RECORD_SIZE) {
            let classic = u16::from_le_bytes([row[0], row[1]]);
            eyre::ensure!(previous.is_none_or(|id| id < classic), "KRLM ids must be strictly ascending: {classic}");
            let material = u16::from_le_bytes([row[2], row[3]]);
            map.insert(classic, KrLand { material, subtype: row[4] });
            previous = Some(classic);
        }
        Ok(map)
    }

    /// Writes sorted KRLM v1 records. The optional fallback is not serialized.
    pub fn to_binary(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(12 + self.len() * 5);
        bytes.extend_from_slice(b"KRLM");
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&5u16.to_le_bytes());
        bytes.extend_from_slice(&(self.len() as u32).to_le_bytes());
        for (classic, material, subtype) in self.rows() {
            bytes.extend_from_slice(&classic.to_le_bytes());
            bytes.extend_from_slice(&material.to_le_bytes());
            bytes.push(subtype);
        }
        bytes
    }

    /// Makes [`Self::square_land`] store `fallback` (with the Classic id itself) for the Classic ids the map lacks,
    /// such as the land tiles added after the KR release, instead of failing.
    pub fn with_fallback(mut self, fallback: KrLand) -> Self {
        self.fallback = Some(fallback);
        self
    }

    pub fn get(&self, classic_land: u16) -> Option<KrLand> {
        self.map.get(&classic_land).copied()
    }

    pub fn insert(&mut self, classic_land: u16, land: KrLand) -> Option<KrLand> {
        self.map.insert(classic_land, land)
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// The rows by Classic id, as `(classic land id, material, subtype)`.
    pub fn rows(&self) -> Vec<(u16, u16, u8)> {
        let mut rows: Vec<_> =
            self.map.iter().map(|(&classic, land)| (classic, land.material, land.subtype)).collect();
        rows.sort_unstable();
        rows
    }

    /// The land id and KR square bytes the encoder stores for `classic_land`. Fails on an id the map lacks, unless
    /// it has a fallback ([`Self::with_fallback`]).
    pub fn square_land(&self, classic_land: u16) -> eyre::Result<(u16, Option<KrSquareLand>)> {
        let land = self
            .get(classic_land)
            .or(self.fallback)
            .ok_or_else(|| eyre!("Classic land id {classic_land} has no Kingdom Reborn material in the land map"))?;
        Ok((land.material, Some(KrSquareLand { subtype: land.subtype, classic_land })))
    }
}

/// Counts the `(material, subtype)` stored with each Classic id over KR sectors, to derive a [`KrLandMap`].
#[derive(Debug, Clone, Default)]
pub struct KrLandCounter {
    material: BTreeMap<u16, BTreeMap<u16, u64>>,
    subtype: BTreeMap<u16, BTreeMap<u8, u64>>,
}

impl KrLandCounter {
    /// Counts the squares of `sector` for which `on_map(square index)` holds (pass the facet geometry check to
    /// leave out the padding squares).
    pub fn add_sector(&mut self, sector: &DecodedEcFacet, mut on_map: impl FnMut(usize) -> bool) {
        for (index, tile) in sector.tiles.iter().enumerate() {
            let Some(kr) = tile.kr else {
                continue;
            };
            if !on_map(index) {
                continue;
            }
            *self.material.entry(kr.classic_land).or_default().entry(tile.land_graphic).or_default() += 1;
            *self.subtype.entry(kr.classic_land).or_default().entry(kr.subtype).or_default() += 1;
        }
    }

    /// The most frequent `material` and `subtype` of each Classic id (ties to the smaller value).
    pub fn to_map(&self) -> KrLandMap {
        fn most_frequent<T: Copy + Ord>(counts: &BTreeMap<T, u64>) -> T {
            // BTreeMap iterates ascending, and max_by_key keeps the last maximum: reverse to keep the smallest.
            *counts.iter().rev().max_by_key(|(_, count)| **count).expect("counted ids have a value").0
        }
        let mut map = KrLandMap::default();
        for (&classic, kr_lands) in &self.material {
            let subtype = most_frequent(&self.subtype[&classic]);
            map.insert(classic, KrLand { material: most_frequent(kr_lands), subtype });
        }
        map
    }

    /// `(squares, squares whose material, squares whose subtype equal the map's)` over the counted squares.
    pub fn agreement(&self, map: &KrLandMap) -> (u64, u64, u64) {
        let (mut total, mut material_ok, mut subtype_ok) = (0, 0, 0);
        for (classic, kr_lands) in &self.material {
            let land = map.get(*classic);
            for (material, count) in kr_lands {
                total += count;
                if land.is_some_and(|land| land.material == *material) {
                    material_ok += count;
                }
            }
            for (subtype, count) in &self.subtype[classic] {
                if land.is_some_and(|land| land.subtype == *subtype) {
                    subtype_ok += count;
                }
            }
        }
        (total, material_ok, subtype_ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_binary_matches_observation_evidence() {
        let mut counter = KrLandCounter::default();
        let evidence = include_str!("../../../assets/kr/observations.csv");
        let mut reader = csv::Reader::from_reader(evidence.as_bytes());
        for row in reader.deserialize::<(u16, u16, u8, u64)>() {
            let (classic, material, subtype, count) = row.unwrap();
            *counter.material.entry(classic).or_default().entry(material).or_default() += count;
            *counter.subtype.entry(classic).or_default().entry(subtype).or_default() += count;
        }
        let verified = KrLandMap::shipped_verified();
        assert_eq!(verified, counter.to_map());
        assert_eq!(counter.agreement(&verified), (69_746_240, 69_745_894, 69_723_228));
        assert_eq!(verified.to_binary(), SHIPPED_KR_LAND_BINARY);
    }

    #[test]
    fn binary_round_trip_preserves_ids_and_excludes_fallback() {
        let mut map = KrLandMap::default();
        map.insert(u16::MAX, KrLand { material: u16::MAX, subtype: u8::MAX });
        map.insert(0, KrLand { material: 7, subtype: 2 });
        let bytes = map.to_binary();
        assert_eq!(KrLandMap::from_binary(&bytes).unwrap(), map);
        assert_eq!(KrLandMap::default(), KrLandMap::from_binary(&KrLandMap::default().to_binary()).unwrap());
        map = map.with_fallback(KrLand { material: 3, subtype: 1 });
        assert_eq!(map.to_binary(), bytes);
        assert!(KrLandMap::from_binary(&bytes).unwrap().square_land(1).is_err());
    }

    #[test]
    fn binary_rejects_invalid_headers_lengths_and_order() {
        let bytes = SHIPPED_KR_LAND_BINARY;
        for len in [0, 4, 11, bytes.len() - 1] {
            assert!(KrLandMap::from_binary(&bytes[..len]).is_err());
        }
        for (offset, value) in [(0, b'X'), (4, 2), (6, 6), (8, 0), (11, 1)] {
            let mut invalid = bytes.to_vec();
            invalid[offset] = value;
            assert!(KrLandMap::from_binary(&invalid).is_err(), "header byte {offset}");
        }
        let mut trailing = bytes.to_vec();
        trailing.push(0);
        assert!(KrLandMap::from_binary(&trailing).is_err());
        let mut duplicate = bytes.to_vec();
        duplicate[17..19].copy_from_slice(&bytes[12..14]);
        assert!(KrLandMap::from_binary(&duplicate).is_err());
        let mut unsorted = bytes.to_vec();
        unsorted[12..17].copy_from_slice(&bytes[17..22]);
        unsorted[17..22].copy_from_slice(&bytes[12..17]);
        assert!(KrLandMap::from_binary(&unsorted).is_err());
    }

    #[test]
    fn tables_are_sorted_and_disjoint() {
        assert!(BUILDER_KR_LAND_MAP_SUPPLEMENT.windows(2).all(|pair| pair[0].0 < pair[1].0));
        let verified = KrLandMap::shipped_verified();
        assert_eq!(verified.len(), 1556);
        for &(classic, _, _) in &BUILDER_KR_LAND_MAP_SUPPLEMENT {
            assert!(verified.get(classic).is_none());
        }
        let map = KrLandMap::shipped();
        assert_eq!(map.len(), verified.len() + BUILDER_KR_LAND_MAP_SUPPLEMENT.len());
    }

    #[test]
    fn square_land_stores_the_classic_id() {
        const CLASSIC_GRASS: u16 = 3;
        let map = KrLandMap::shipped();
        let (material, kr) = map.square_land(CLASSIC_GRASS).unwrap();
        assert_eq!(material, map.get(CLASSIC_GRASS).unwrap().material);
        assert_eq!(kr.unwrap().classic_land, CLASSIC_GRASS);
        assert!(KrLandMap::default().square_land(CLASSIC_GRASS).is_err());
        let fallback = KrLand { material: 0, subtype: 0 };
        let (material, kr) = KrLandMap::default().with_fallback(fallback).square_land(CLASSIC_GRASS).unwrap();
        assert_eq!((material, kr.unwrap().classic_land), (fallback.material, CLASSIC_GRASS));
    }

    #[test]
    fn counter_takes_the_most_frequent_value() {
        let mut sector = crate::enhanced::facet_sector::empty_sector(
            0,
            0,
            crate::enhanced::client_dialect::ClientDialect::KingdomReborn,
        )
        .unwrap();
        for (index, tile) in sector.tiles.iter_mut().take(3).enumerate() {
            tile.land_graphic = if index == 0 { 9 } else { 8 };
            tile.kr = Some(KrSquareLand { subtype: if index == 2 { 1 } else { 2 }, classic_land: 5 });
        }
        let mut counter = KrLandCounter::default();
        counter.add_sector(&sector, |index| index < 3);
        let map = counter.to_map();
        assert_eq!(map.get(5), Some(KrLand { material: 8, subtype: 2 }));
        assert_eq!(counter.agreement(&map), (3, 2, 2));
    }
}
