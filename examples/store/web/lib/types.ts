export type Product = {
  id: string;
  name: string;
  description: string;
  brand: string;
  category: string;
  color: string;
  price: number;
  rating: number;
  stock: number;
  imageUrl?: string;
  createdAt: string;
};

export type CartItem = {
  id: string;
  userId: string;
  productId: string;
  productName: string;
  productBrand: string;
  productPrice: number;
  quantity: number;
  addedAt: string;
};

export type Address = {
  id: string;
  userId: string;
  fullName: string;
  street: string;
  city: string;
  postal: string;
  country: string;
  isDefault: boolean;
};

export type OrderStatus = "placed" | "packed" | "shipped" | "delivered";

export type Order = {
  id: string;
  userId: string;
  status: OrderStatus;
  subtotal: number;
  itemCount: number;
  shipName: string;
  shipStreet: string;
  shipCity: string;
  shipPostal: string;
  shipCountry: string;
  placedAt: string;
  trackingNumber: string;
  estimatedDelivery: string;
};

export type OrderItem = {
  id: string;
  orderId: string;
  userId: string;
  productId: string;
  productName: string;
  productBrand: string;
  unitPrice: number;
  quantity: number;
};

export type SearchResult = {
  hits: Product[];
  total: number;
  facet_counts: Record<string, Record<string, number>>;
  took_ms: number;
};

export const STATUS_STEPS: OrderStatus[] = [
  "placed",
  "packed",
  "shipped",
  "delivered",
];

export const STATUS_LABELS: Record<OrderStatus, string> = {
  placed: "Order placed",
  packed: "Packed",
  shipped: "In transit",
  delivered: "Delivered",
};
